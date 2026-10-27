//! Отряды: солдаты команды разбиты на группы с командиром. Командир выбирает,
//! к какой точке идёт весь отряд, и ведёт его по полю путей; остальные держат
//! своё место в строю вокруг него. Отбившиеся (возродились далеко, обходят дом)
//! идут к той же точке сами и встают в строй, когда догонят.
//!
//! Строй считается раз в тик для всех отрядов (это дёшево: отряд — десяток
//! солдат), а двигаются солдаты потом параллельно, каждый по своему приказу.

use crate::map::Map;
use crate::world::{Controller, Soldier};
use glam::Vec2;
use serde::Deserialize;

/// Солдат вне отряда (отряды выключены).
pub const NO_SQUAD: u32 = u32::MAX;

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct SquadSpec {
    /// Солдат в отряде. 0 — без отрядов: каждый сам по себе, как раньше.
    pub size: u32,
    /// Расстояние между соседями в строю, м.
    pub spacing: f32,
    /// Боец ввязывается в перестрелку, только если он не дальше этого от своего
    /// места в строю; иначе догоняет отряд, отстреливаясь на ходу.
    pub leash: f32,
    /// Дальше этого от места в строю боец не пытается идти к нему напрямую,
    /// а идёт к точке отряда по полю путей.
    pub rejoin: f32,
}

impl Default for SquadSpec {
    fn default() -> Self {
        SquadSpec { size: 6, spacing: 3.0, leash: 16.0, rejoin: 25.0 }
    }
}

#[derive(Clone, Debug)]
pub struct Squad {
    pub team: u8,
    /// Номера солдат. Командир — первый живой из них под управлением ИИ.
    pub members: Vec<u32>,
    pub leader: Option<u32>,
    /// Точка захвата, к которой идёт отряд (`NO_OBJECTIVE` — ещё не выбрал).
    pub objective: u32,
    /// Куда смотрит строй — сглаженное направление движения командира.
    pub heading: Vec2,
}

/// Что делать солдату в этом тике.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum Order {
    /// Сам по себе: игрок, солдат без отряда, отряд без командира.
    #[default]
    Free,
    /// Командир: ведёт отряд к точке; `pace` — доля скорости (ждёт отставших).
    Lead { pace: f32 },
    /// Боец: держать место `slot`, двигаясь вместе с командиром.
    Follow { slot: Vec2, leader_vel: Vec2, heading: Vec2 },
}

/// Шаг командира: чуть медленнее бойцов, чтобы строй успевал. Если кто-то
/// отстал — ещё медленнее.
const PACE: f32 = 0.85;
const PACE_WAITING: f32 = 0.55;
/// Боец «отстал», если он дальше этого от своего места (но ещё в пределах `rejoin`).
const LAGGING: f32 = 8.0;

/// Делит солдат каждой команды на отряды по порядку номеров.
pub fn form(soldiers: &mut [Soldier], size: u32) -> Vec<Squad> {
    let mut squads: Vec<Squad> = Vec::new();
    if size == 0 {
        soldiers.iter_mut().for_each(|s| s.squad = NO_SQUAD);
        return squads;
    }
    for team in 0..2u8 {
        let mut current: Option<usize> = None;
        for (i, s) in soldiers.iter_mut().enumerate().filter(|(_, s)| s.team == team) {
            let index = match current {
                Some(c) if squads[c].members.len() < size as usize => c,
                _ => {
                    squads.push(Squad { team, members: Vec::new(), leader: None, objective: crate::world::NO_OBJECTIVE, heading: Vec2::ZERO });
                    squads.len() - 1
                }
            };
            current = Some(index);
            squads[index].members.push(i as u32);
            s.squad = index as u32;
        }
    }
    squads
}

/// Место `rank`-го бойца (с 1) относительно командира: `x` — вперёд, `y` — влево.
/// В движении — клин за командиром, на точке — широкая цепь.
pub fn formation_offset(rank: u32, spacing: f32, on_point: bool) -> Vec2 {
    let row = rank.div_ceil(2) as f32;
    let side = if rank % 2 == 1 { 1.0 } else { -1.0 };
    if on_point {
        Vec2::new(0.0, side * row * spacing * 1.6)
    } else {
        Vec2::new(-row * spacing, side * row * spacing)
    }
}

/// Командиры, направление строя и приказ каждому солдату на этот тик.
/// `points` — где точки захвата, `radius` — радиус точки.
/// `body` — радиус солдата: место в строю не должно задевать стену.
#[allow(clippy::too_many_arguments)]
pub(crate) fn plan(squads: &mut [Squad], soldiers: &[Soldier], orders: &mut Vec<Order>, map: &Map, spec: &SquadSpec, points: &[Vec2], radius: f32, body: f32) {
    orders.clear();
    orders.resize(soldiers.len(), Order::Free);
    let ai_alive = |i: u32| {
        let s = &soldiers[i as usize];
        s.alive && s.controller == Controller::Ai
    };
    for squad in squads.iter_mut() {
        // Командир остаётся, пока жив; иначе — следующий по списку.
        if !squad.leader.is_some_and(ai_alive) {
            squad.leader = squad.members.iter().copied().find(|&i| ai_alive(i));
        }
        let Some(leader) = squad.leader else { continue };
        let l = &soldiers[leader as usize];
        let target = points.get(squad.objective as usize).copied();
        // Строй смотрит туда, куда идёт командир; пока стоит — к точке.
        let along = if l.vel.length_squared() > 0.25 {
            l.vel.normalize()
        } else {
            target.map_or(Vec2::ZERO, |t| (t - l.pos).normalize_or_zero())
        };
        squad.heading = if squad.heading == Vec2::ZERO { along } else { (squad.heading * 0.85 + along * 0.15).normalize_or(squad.heading) };
        let forward = if squad.heading == Vec2::ZERO { Vec2::X } else { squad.heading };
        let left = forward.perp();
        let on_point = target.is_some_and(|t| t.distance(l.pos) < radius);

        let mut lagging = false;
        let mut rank = 0;
        for &m in &squad.members {
            if m == leader || !ai_alive(m) {
                continue;
            }
            rank += 1;
            let o = formation_offset(rank, spec.spacing, on_point);
            let mut slot = l.pos + forward * o.x + left * o.y;
            // Место в стене — подвигаем ближе к командиру.
            for k in [0.75, 0.5, 0.25, 0.0] {
                if !map.circle_hits_solid(slot, body) {
                    break;
                }
                slot = l.pos + (forward * o.x + left * o.y) * k;
            }
            let d = soldiers[m as usize].pos.distance(slot);
            lagging |= d > LAGGING && d < spec.rejoin;
            orders[m as usize] = Order::Follow { slot, leader_vel: l.vel, heading: forward };
        }
        orders[leader as usize] = Order::Lead { pace: if lagging { PACE_WAITING } else { PACE } };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{SimConfig, World};

    fn world(per_team: u32) -> World {
        World::new(SimConfig { map_size: 256, soldiers_per_team: per_team, capture_points: 3, ..Default::default() }, 3)
    }

    #[test]
    fn every_soldier_is_in_one_squad_of_his_team() {
        let w = world(20);
        let size = w.config.squad.size as usize;
        let mut seen = vec![0; w.soldiers.len()];
        for (index, squad) in w.squads.iter().enumerate() {
            assert!(!squad.members.is_empty() && squad.members.len() <= size);
            for &m in &squad.members {
                let s = &w.soldiers[m as usize];
                assert_eq!(s.team, squad.team);
                assert_eq!(s.squad, index as u32);
                seen[m as usize] += 1;
            }
        }
        assert!(seen.iter().all(|&n| n == 1));
        // 20 солдат по 6 — четыре отряда в команде (6, 6, 6, 2).
        assert_eq!(w.squads.len(), 8);
    }

    #[test]
    fn wedge_behind_the_leader_line_on_the_point() {
        for rank in 1..6 {
            let o = formation_offset(rank, 3.0, false);
            assert!(o.x < 0.0, "в клине бойцы позади командира");
            assert!(o.y != 0.0);
            assert_eq!(formation_offset(rank, 3.0, true).x, 0.0, "на точке — цепь рядом с командиром");
        }
        // Пары — по обе стороны.
        assert_eq!(formation_offset(1, 3.0, false).y, -formation_offset(2, 3.0, false).y);
    }

    #[test]
    fn next_soldier_leads_when_the_leader_falls() {
        let mut w = world(12);
        w.step();
        let first = w.squads[0].leader.unwrap();
        assert_eq!(first, w.squads[0].members[0]);
        assert!(matches!(w.orders[first as usize], Order::Lead { .. }));
        w.kill(first, w.squads.last().unwrap().members[0]);
        w.step();
        let next = w.squads[0].leader.unwrap();
        assert_ne!(next, first);
        assert_eq!(next, w.squads[0].members[1]);
        // У остальных — места в строю.
        assert!(w.squads[0].members[2..].iter().all(|&m| matches!(w.orders[m as usize], Order::Follow { .. })));
    }

    #[test]
    fn fallen_soldier_returns_near_his_leader() {
        let mut w = world(12);
        w.step();
        let (leader, member, team) = (w.squads[0].leader.unwrap(), w.squads[0].members[1], w.squads[0].team);
        let enemy = w.squads.iter().find(|q| q.team != team).unwrap().members[0];
        // Командир стоит на нашей точке A, далеко от базы.
        let a = w.points[0].pos;
        w.kill(member, enemy);
        for _ in 0..(w.config.soldier.respawn_time * 20.0) as u32 + 5 {
            w.points[0].owner = Some(team);
            w.points[0].progress = if team == 0 { -1.0 } else { 1.0 };
            w.soldiers[leader as usize].pos = a;
            w.step();
        }
        let s = &w.soldiers[member as usize];
        assert!(s.alive);
        assert!(s.pos.distance(a) < 10.0, "появился у точки командира, а не на базе: {:?} vs {a:?}", s.pos);
    }

    #[test]
    fn player_is_never_the_leader() {
        let mut w = world(12);
        let me = w.take_control(0).unwrap();
        w.step();
        let squad = &w.squads[w.soldiers[me as usize].squad as usize];
        assert_ne!(squad.leader, Some(me));
        assert_eq!(w.orders[me as usize], Order::Free);
    }

    #[test]
    fn no_squads_when_size_is_zero() {
        let mut c = SimConfig { map_size: 256, soldiers_per_team: 10, capture_points: 3, ..Default::default() };
        c.squad.size = 0;
        let mut w = World::new(c, 3);
        w.step();
        assert!(w.squads.is_empty());
        assert!(w.soldiers.iter().all(|s| s.squad == NO_SQUAD));
        assert!(w.orders.iter().all(|o| *o == Order::Free));
    }
}
