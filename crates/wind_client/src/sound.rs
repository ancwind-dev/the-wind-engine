//! Звук боя: события мира → голоса микшера. Слушатель — свой солдат (или центр
//! камеры у наблюдателя), лево-право — по экрану. Врагов за туманом войны не видно,
//! но слышно.

use crate::source::View;
use std::collections::HashSet;
use wind_audio::mixer::spatial;
use wind_audio::{Audio, Gun, Play, Sfx};
use wind_net::replication::Heard;
use wind_sim::Vec2;
use wind_sim::arsenal::WeaponSpec;
use wind_sim::round::Phase;

/// Сколько новых голосов за кадр, не больше: в бою на 600 солдат выстрелов сотни,
/// слышны всё равно самые громкие.
const PER_FRAME: usize = 24;
/// Шаг своего солдата, м.
const STRIDE: f32 = 1.6;
/// Пуля «свистит», если проходит ближе этого, м.
const WHIZ_DISTANCE: f32 = 3.0;

/// Звук выстрела оружия: из `sound` в `weapons.ron`, иначе по виду оружия.
pub fn gun(spec: &WeaponSpec) -> Gun {
    match spec.sound.as_str() {
        "" if spec.blast.is_some() => Gun::Launcher,
        "" => Gun::Rifle,
        name => Gun::from_name(name),
    }
}

/// Кадр для звука: где слушатель и что вокруг.
pub struct Ears {
    pub pos: Vec2,
    /// Вправо по экрану.
    pub right: Vec2,
    /// Свой солдат (его выстрелы и шаги — «у уха»).
    pub own_id: Option<u32>,
    pub own_pos: Option<Vec2>,
}

#[derive(Default)]
pub struct Sound {
    owners: Vec<Option<u8>>,
    round_over: Option<bool>,
    reloading: bool,
    walked: f32,
    last_pos: Option<Vec2>,
    /// Пули, что уже просвистели (по скорости — она у каждой своя).
    whizzed: HashSet<(u32, u32)>,
    variant: u32,
}

impl Sound {
    fn next_variant(&mut self) -> u32 {
        self.variant = self.variant.wrapping_add(1);
        self.variant
    }

    /// Что сыграть в этом кадре. `gun_of(команда, стрелок)` — звук его оружия.
    pub fn frame(&mut self, ears: &Ears, view: &View, heard: &[Heard], crumbled: &[Vec2], gun_of: impl Fn(u8, u32) -> Gun) -> Vec<Play> {
        let mut plays = Vec::new();
        let at = |s: &mut Self, plays: &mut Vec<Play>, sfx: Sfx, pos: Vec2, range: f32, gain: f32, extra_delay: f32| {
            if let Some((g, pan, delay)) = spatial(ears.pos, ears.right, pos, range) {
                let variant = s.next_variant();
                plays.push(Play { sfx, variant, gain: g * gain, pan, delay: delay + extra_delay });
            }
        };
        for h in heard {
            match *h {
                Heard::Shot { shooter, .. } if Some(shooter) == ears.own_id => {
                    let v = self.next_variant();
                    plays.push(Play::flat(Sfx::Shot(gun_of(view.round.own_team.unwrap_or(0), shooter)), v, 0.5));
                }
                Heard::Shot { pos, shooter, team } => at(self, &mut plays, Sfx::Shot(gun_of(team, shooter)), pos, 16.0, 0.75, 0.0),
                Heard::Throw { pos } => at(self, &mut plays, Sfx::Throw, pos, 4.0, 0.6, 0.0),
                Heard::Blast { pos, .. } => at(self, &mut plays, Sfx::Explosion, pos, 35.0, 1.0, 0.0),
            }
        }
        for &p in crumbled {
            at(self, &mut plays, Sfx::Crumble, p, 20.0, 0.7, 0.08);
        }
        self.whiz(ears, view, &mut plays);
        self.own(ears, view, &mut plays);
        self.match_events(view, &mut plays);
        plays.sort_by(|a, b| b.gain.total_cmp(&a.gain));
        plays.truncate(PER_FRAME);
        plays
    }

    /// Свист пуль, которые вот-вот пройдут рядом. Чужие пули — свои уходят от нас.
    fn whiz(&mut self, ears: &Ears, view: &View, plays: &mut Vec<Play>) {
        let key = |v: Vec2| (v.x.to_bits(), v.y.to_bits());
        self.whizzed.retain(|k| view.bullets.iter().any(|&(_, v)| key(v) == *k));
        for &(p, v) in &view.bullets {
            let speed2 = v.length_squared();
            if speed2 < 1.0 || self.whizzed.contains(&key(v)) {
                continue;
            }
            // Через сколько секунд пуля ближе всего к нам и насколько близко.
            let s = (ears.pos - p).dot(v) / speed2;
            let ahead = s * speed2.sqrt();
            if !(0.0..=8.0).contains(&ahead) || p.distance(ears.pos) < 1.5 {
                continue;
            }
            let closest = p + v * s;
            let miss = closest.distance(ears.pos);
            if miss > WHIZ_DISTANCE {
                continue;
            }
            self.whizzed.insert(key(v));
            let side = (closest - ears.pos).dot(ears.right).signum() * 0.6;
            let variant = self.next_variant();
            // Звук длиной 0,32 с, громче всего посередине — середина совпадёт с пролётом.
            plays.push(Play { sfx: Sfx::Whiz, variant, gain: 0.5 * (1.0 - miss / WHIZ_DISTANCE * 0.6), pan: side, delay: (s - 0.16).max(0.0) });
        }
    }

    /// Свои шаги и перезарядка.
    fn own(&mut self, ears: &Ears, view: &View, plays: &mut Vec<Play>) {
        let alive = view.me.alive && view.has_me;
        match (ears.own_pos.filter(|_| alive), self.last_pos) {
            (Some(p), Some(last)) => {
                let d = p.distance(last);
                // Прыжок — возрождение или поправка от сервера, а не шаг.
                if d < 2.0 {
                    self.walked += d;
                }
                if self.walked >= STRIDE {
                    self.walked -= STRIDE;
                    let v = self.next_variant();
                    plays.push(Play::flat(Sfx::Step, v, 0.22));
                }
            }
            _ => self.walked = STRIDE * 0.5,
        }
        self.last_pos = ears.own_pos.filter(|_| alive);
        let reloading = alive && view.me.reloading;
        if reloading && !self.reloading {
            plays.push(Play::flat(Sfx::Reload, 0, 0.4));
        }
        self.reloading = reloading;
    }

    /// Точки и раунд — сигналы интерфейса, без места.
    fn match_events(&mut self, view: &View, plays: &mut Vec<Play>) {
        let team = view.round.own_team.filter(|_| view.has_me);
        if self.owners.len() == view.owners.len() {
            for (&was, &now) in self.owners.iter().zip(&view.owners) {
                if was == now {
                    continue;
                }
                match team {
                    Some(t) if now == Some(t) => plays.push(Play::flat(Sfx::Captured, 0, 0.45)),
                    Some(t) if was == Some(t) => plays.push(Play::flat(Sfx::Lost, 0, 0.45)),
                    _ => {}
                }
            }
        }
        self.owners = view.owners.clone();
        let over = match &view.round.round.phase {
            Phase::Playing => None,
            Phase::Over { winner, .. } => Some(*winner),
        };
        // Первый кадр (подключились в перерыве) — молчим.
        if let Some(was_over) = self.round_over
            && !was_over
            && let Some(winner) = over
        {
            let won = team.is_none() || winner == team;
            plays.push(Play::flat(if won { Sfx::Win } else { Sfx::Lose }, 0, 0.5));
        }
        self.round_over = Some(over.is_some());
    }
}

/// Сыграть кадр.
pub fn play_all(audio: &Audio, plays: Vec<Play>) {
    for p in plays {
        audio.play(p);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wind_sim::round::{EndReason, Round};

    fn ears() -> Ears {
        Ears { pos: Vec2::ZERO, right: Vec2::X, own_id: Some(0), own_pos: Some(Vec2::ZERO) }
    }

    #[test]
    fn bullet_passing_close_whizzes_once() {
        let mut sound = Sound::default();
        let view = View { bullets: vec![(Vec2::new(-5.0, 1.0), Vec2::new(90.0, 0.0)), (Vec2::new(-5.0, 9.0), Vec2::new(90.0, 0.0001))], ..Default::default() };
        let first = sound.frame(&ears(), &view, &[], &[], |_, _| Gun::Rifle);
        assert_eq!(first.iter().filter(|p| p.sfx == Sfx::Whiz).count(), 1, "далёкая пуля не свистит");
        // Та же пуля в следующем кадре (симуляция ещё не сдвинула её) — не повторяем.
        let again = sound.frame(&ears(), &view, &[], &[], |_, _| Gun::Rifle);
        assert!(again.iter().all(|p| p.sfx != Sfx::Whiz));
        // Своя пуля, летящая от нас, — тоже нет.
        let mut sound = Sound::default();
        let own = View { bullets: vec![(Vec2::new(0.5, 0.0), Vec2::new(90.0, 0.0))], ..Default::default() };
        assert!(sound.frame(&ears(), &own, &[], &[], |_, _| Gun::Rifle).is_empty());
    }

    #[test]
    fn capture_and_round_end_ring() {
        let mut sound = Sound::default();
        let mut view = View { owners: vec![None, Some(1)], has_me: true, ..Default::default() };
        view.round.own_team = Some(0);
        assert!(sound.frame(&ears(), &view, &[], &[], |_, _| Gun::Rifle).is_empty(), "первый кадр — тишина");
        view.owners = vec![Some(0), None];
        let plays = sound.frame(&ears(), &view, &[], &[], |_, _| Gun::Rifle);
        assert!(plays.iter().any(|p| p.sfx == Sfx::Captured));
        assert!(plays.iter().all(|p| p.sfx != Sfx::Lost), "чужая точка ушла — не наша потеря");
        view.round.round = Round { phase: Phase::Over { winner: Some(1), reason: EndReason::TimeUp }, ..Default::default() };
        let plays = sound.frame(&ears(), &view, &[], &[], |_, _| Gun::Rifle);
        assert!(plays.iter().any(|p| p.sfx == Sfx::Lose));
    }

    #[test]
    fn own_shots_are_close_and_others_far_quieter() {
        let mut sound = Sound::default();
        let heard = [
            Heard::Shot { pos: Vec2::ZERO, shooter: 0, team: 0 },
            Heard::Shot { pos: Vec2::new(60.0, 0.0), shooter: 5, team: 1 },
        ];
        let plays = sound.frame(&ears(), &View::default(), &heard, &[], |t, _| if t == 1 { Gun::Smg } else { Gun::Rifle });
        assert_eq!(plays.len(), 2);
        assert_eq!(plays[0].sfx, Sfx::Shot(Gun::Rifle));
        assert_eq!(plays[0].pan, 0.0);
        assert_eq!(plays[1].sfx, Sfx::Shot(Gun::Smg));
        assert!(plays[1].gain < plays[0].gain * 0.5 && plays[1].pan > 0.5 && plays[1].delay > 0.1);
    }
}
