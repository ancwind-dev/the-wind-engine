//! Игрок-командир и звания, как в RWR: за убийства и захват точек растёт опыт, с ним —
//! звание, а со званием — сколько бойцов игрок может вести за собой.
//!
//! Бойцов игрок набирает сам — из своих солдат под управлением ИИ: наводит на бойца
//! прицел и зовёт (E). Отряд игрока идёт за ним клином («за мной», F) или держит точку,
//! на которую указал прицел («держать», Q). X — распустить отряд. Набранный боец выходит
//! из своего отряда и из-под штаба (`Soldier::commander`); погиб — выбывает, распустили —
//! возвращается в свой отряд.

use crate::squad::{Order, formation_offset};
use crate::world::{Controller, NO_OBJECTIVE, World};
use glam::Vec2;
use serde::Deserialize;

/// Солдат не в отряде игрока.
pub const NO_COMMANDER: u32 = u32::MAX;

/// Звание: с какого опыта и сколько бойцов можно вести.
#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(default)]
pub struct Rank {
    pub name: String,
    pub xp: u32,
    pub squad: u32,
}

impl Default for Rank {
    fn default() -> Self {
        Rank { name: "Рядовой".into(), xp: 0, squad: 0 }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct RankSpec {
    /// Звания по возрастанию опыта; первое — с нуля.
    pub ranks: Vec<Rank>,
    /// Опыт за убитого врага и за захват точки (тем, кто стоял на ней).
    pub kill_xp: u32,
    pub capture_xp: u32,
    /// Бойца зовут, если он не дальше `recruit_radius` от прицела и не дальше
    /// `recruit_reach` от самого игрока, м.
    pub recruit_radius: f32,
    pub recruit_reach: f32,
}

impl Default for RankSpec {
    fn default() -> Self {
        let rank = |name: &str, xp, squad| Rank { name: name.into(), xp, squad };
        RankSpec {
            ranks: vec![
                rank("Рядовой", 0, 0),
                rank("Ефрейтор", 3, 1),
                rank("Младший сержант", 8, 2),
                rank("Сержант", 15, 3),
                rank("Старший сержант", 25, 4),
                rank("Старшина", 40, 5),
                rank("Прапорщик", 60, 6),
                rank("Лейтенант", 85, 8),
                rank("Капитан", 120, 10),
            ],
            kill_xp: 1,
            capture_xp: 3,
            recruit_radius: 4.0,
            recruit_reach: 40.0,
        }
    }
}

impl RankSpec {
    /// Номер звания при опыте `xp`.
    pub fn rank_of(&self, xp: u32) -> usize {
        self.ranks.iter().rposition(|r| xp >= r.xp).unwrap_or(0)
    }

    /// Сколько бойцов можно вести при опыте `xp`.
    pub fn squad_size(&self, xp: u32) -> u32 {
        self.ranks.get(self.rank_of(xp)).map_or(0, |r| r.squad)
    }
}

/// Приказ игрока своему отряду (`PlayerInput::command`). Все приказы можно
/// повторять: клиент держит приказ несколько тиков, чтобы он не потерялся в сети.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Command {
    #[default]
    None,
    /// Позвать в отряд бойца у прицела.
    Recruit,
    /// Распустить отряд.
    Dismiss,
    /// За мной.
    Follow,
    /// Держать точку у прицела.
    Hold,
}

impl Command {
    pub fn to_u8(self) -> u8 {
        self as u8
    }

    pub fn from_u8(v: u8) -> Command {
        match v {
            1 => Command::Recruit,
            2 => Command::Dismiss,
            3 => Command::Follow,
            4 => Command::Hold,
            _ => Command::None,
        }
    }
}

/// Отряд игрока, держащий точку: где и куда смотрит.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hold {
    pub at: Vec2,
    pub facing: Vec2,
}

impl World {
    /// Бойцы игрока `player` по порядку номеров (живые).
    pub fn squad_of(&self, player: u32) -> Vec<u32> {
        (0..self.soldiers.len() as u32).filter(|&i| self.soldiers[i as usize].commander == player && self.soldiers[i as usize].alive).collect()
    }

    /// Приказы игроков в этом тике. Сетка солдат уже построена.
    pub(crate) fn apply_commands(&mut self) {
        for p in 0..self.soldiers.len() {
            let s = &self.soldiers[p];
            let Controller::Player(input) = s.controller else { continue };
            if !s.alive {
                continue;
            }
            match input.command {
                Command::None => {}
                Command::Recruit => self.recruit(p as u32, input.aim),
                Command::Dismiss => self.dismiss(p as u32),
                Command::Follow => self.soldiers[p].hold = None,
                Command::Hold => {
                    let facing = (input.aim - s.pos).normalize_or(Vec2::from_angle(s.facing));
                    self.soldiers[p].hold = Some(Hold { at: input.aim, facing });
                }
            }
        }
    }

    /// Позвать бойца, ближайшего к прицелу: своего, под управлением ИИ, не раненого.
    /// Если ближайший уже в отряде — ничего (приказ повторяется несколько тиков).
    fn recruit(&mut self, player: u32, at: Vec2) {
        let spec = &self.config.ranks;
        let me = &self.soldiers[player as usize];
        if self.squad_of(player).len() as u32 >= spec.squad_size(me.xp) {
            return;
        }
        let (r, reach) = (spec.recruit_radius, spec.recruit_reach);
        let mut best: Option<(f32, u32)> = None;
        let soldiers = &self.soldiers;
        self.grid.query(at, r, |j| {
            let s = &soldiers[j as usize];
            let fits = j != player
                && s.alive
                && s.team == me.team
                && s.controller == Controller::Ai
                && s.retreat.is_none()
                && (s.commander == NO_COMMANDER || s.commander == player)
                && s.pos.distance(me.pos) <= reach;
            let d = s.pos.distance_squared(at);
            if fits && d < r * r && best.is_none_or(|(b, _)| d < b) {
                best = Some((d, j));
            }
        });
        if let Some((_, j)) = best
            && self.soldiers[j as usize].commander == NO_COMMANDER
        {
            let s = &mut self.soldiers[j as usize];
            s.commander = player;
            s.cover = None;
        }
    }

    /// Распустить отряд игрока: бойцы возвращаются в свои отряды.
    pub(crate) fn dismiss(&mut self, player: u32) {
        for s in &mut self.soldiers {
            if s.commander == player {
                s.commander = NO_COMMANDER;
                s.objective = NO_OBJECTIVE;
            }
        }
        if let Some(p) = self.soldiers.get_mut(player as usize) {
            p.hold = None;
        }
    }

    /// Места в строю бойцов игроков: клином за игроком или цепью у точки, которую
    /// держат. Идут к ближайшей к игроку (или к удерживаемой) точке захвата, пока
    /// строй далеко или за стенами. Игрок погиб — бойцы без приказа идут туда же.
    pub(crate) fn plan_player_squads(&mut self) {
        let spec = &self.config.squad;
        let body = self.config.soldier.radius;
        let mut members: Vec<(u32, Vec<u32>)> = Vec::new();
        for (i, s) in self.soldiers.iter().enumerate() {
            if s.commander == NO_COMMANDER || !s.alive {
                continue;
            }
            match members.iter_mut().find(|(p, _)| *p == s.commander) {
                Some((_, list)) => list.push(i as u32),
                None => members.push((s.commander, vec![i as u32])),
            }
        }
        for (player, list) in members {
            let l = &self.soldiers[player as usize];
            // Строй смотрит туда, куда игрок идёт; стоит — куда смотрел, пока шёл.
            let heading = if l.vel.length_squared() > 1.0 { l.vel.normalize() } else { l.squad_heading };
            let heading = if heading == Vec2::ZERO { Vec2::from_angle(l.facing) } else { heading };
            let (anchor, forward, on_point, vel) = match l.hold {
                Some(h) => (h.at, h.facing, true, Vec2::ZERO),
                None => (l.pos, heading, false, l.vel),
            };
            let alive = l.alive;
            self.soldiers[player as usize].squad_heading = heading;
            let goal = self.points.iter().enumerate().min_by(|a, b| a.1.pos.distance_squared(anchor).total_cmp(&b.1.pos.distance_squared(anchor))).map_or(NO_OBJECTIVE, |(i, _)| i as u32);
            for (k, &m) in list.iter().enumerate() {
                self.soldiers[m as usize].objective = goal;
                if !alive {
                    continue;
                }
                let o = formation_offset(k as u32 + 1, spec.spacing, on_point);
                let left = forward.perp();
                let mut slot = anchor + forward * o.x + left * o.y;
                for f in [0.75, 0.5, 0.25, 0.0] {
                    if !self.map.circle_hits_solid(slot, body) {
                        break;
                    }
                    slot = anchor + (forward * o.x + left * o.y) * f;
                }
                self.orders[m as usize] = Order::Follow { slot, leader_vel: vel, heading: forward, leader: anchor };
            }
        }
    }

    /// Опыт за захват точки — тем из команды, кто стоял на ней.
    pub(crate) fn reward_capture(&mut self, point: usize, team: u8) {
        let (at, r, xp) = (self.points[point].pos, self.config.capture.radius, self.config.ranks.capture_xp);
        for s in &mut self.soldiers {
            if s.alive && s.team == team && s.pos.distance_squared(at) < r * r {
                s.xp += xp;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PlayerInput, SimConfig};

    #[test]
    fn fallen_member_leaves_and_player_who_left_dismisses_his_squad() {
        let mut w = World::new(SimConfig { map_size: 256, soldiers_per_team: 8, capture_points: 3, ..Default::default() }, 2);
        let me = w.take_control(0).unwrap();
        w.soldiers[me as usize].xp = 1000;
        let friends: Vec<u32> = (0..w.soldiers.len() as u32).filter(|&i| i != me && w.soldiers[i as usize].team == 0).take(3).collect();
        for &f in &friends {
            w.soldiers[f as usize].pos = w.soldiers[me as usize].pos + Vec2::new(2.0, 0.0);
            let aim = w.soldiers[f as usize].pos;
            w.set_input(me, PlayerInput { aim, command: Command::Recruit, ..Default::default() });
            w.step();
            w.soldiers[f as usize].pos = w.soldiers[me as usize].pos + Vec2::new(-4.0, f as f32);
        }
        assert_eq!(w.squad_of(me).len(), 3);
        let enemy = (0..w.soldiers.len() as u32).find(|&i| w.soldiers[i as usize].team == 1).unwrap();
        w.kill(friends[0], enemy, Vec2::ZERO, false);
        assert_eq!(w.soldiers[friends[0] as usize].commander, NO_COMMANDER, "погибший выбыл");
        assert_eq!(w.squad_of(me).len(), 2);
        // Опыт: за врага — да, за своего — нет; за захват — тем, кто на точке.
        let xp = w.soldiers[me as usize].xp;
        w.kill(enemy, me, Vec2::ZERO, false);
        w.kill(friends[1], me, Vec2::ZERO, false);
        assert_eq!(w.soldiers[me as usize].xp, xp + w.config.ranks.kill_xp);
        w.points[0].pos = w.soldiers[me as usize].pos;
        w.reward_capture(0, 0);
        assert_eq!(w.soldiers[me as usize].xp, xp + w.config.ranks.kill_xp + w.config.ranks.capture_xp);
        // Игрок ушёл — отряд распущен.
        w.release_control(me);
        assert!(w.soldiers.iter().all(|s| s.commander == NO_COMMANDER));
    }
}

