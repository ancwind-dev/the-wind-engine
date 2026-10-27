//! Туман войны на сервере: кого из врагов видит каждая команда. Клиенту шлём своих
//! и только тех врагов, кого видит его команда, — нечестный клиент не узнает, кто
//! стоит за стеной (раньше сервер слал всех в радиусе интереса, а прятал их клиент).
//!
//! Правило то же, что у тумана на клиенте: врага видно, если его видит кто-то из своих —
//! в пределах дальности обзора и не сквозь стену. Проверяем до `OBSERVERS` ближайших
//! своих. Замеченный враг остаётся видимым ещё `LINGER` тиков: иначе он мигал бы,
//! перебегая мимо угла.

use rayon::prelude::*;
use wind_sim::World;

/// Сколько ближайших своих проверять на прямую видимость.
const OBSERVERS: usize = 6;
/// Сколько тиков враг остаётся видимым после того, как пропал из виду (0,5 с).
pub const LINGER: u64 = 10;

#[derive(Default)]
pub struct Vision {
    /// До какого тика солдат виден команде: `[команда][солдат]`.
    seen_until: [Vec<u64>; 2],
}

impl Vision {
    /// Пересчитать видимость после тика.
    pub fn update(&mut self, world: &World) {
        let n = world.soldiers.len();
        let range = world.config.soldier.vision_range;
        let tick = world.tick;
        for team in 0..2u8 {
            let seen = &mut self.seen_until[team as usize];
            seen.resize(n, 0);
            seen.par_iter_mut().enumerate().for_each(|(i, until)| {
                let s = &world.soldiers[i];
                if !s.alive || s.team == team {
                    return;
                }
                // Ближайшие свои в пределах обзора — по возрастанию расстояния.
                let mut near: [(f32, u32); OBSERVERS] = [(f32::INFINITY, 0); OBSERVERS];
                world.soldiers_near(s.pos, range, |j| {
                    let o = &world.soldiers[j as usize];
                    if o.team != team || !o.alive {
                        return;
                    }
                    let d2 = o.pos.distance_squared(s.pos);
                    if d2 >= range * range || d2 >= near[OBSERVERS - 1].0 {
                        return;
                    }
                    let mut k = OBSERVERS - 1;
                    while k > 0 && near[k - 1].0 > d2 {
                        near[k] = near[k - 1];
                        k -= 1;
                    }
                    near[k] = (d2, j);
                });
                let visible = near
                    .iter()
                    .take_while(|(d2, _)| d2.is_finite())
                    .any(|&(_, j)| world.map.line_of_sight(world.soldiers[j as usize].pos, s.pos));
                if visible {
                    *until = tick + LINGER;
                }
            });
        }
    }

    /// Видит ли команда `team` солдата `soldier` сейчас (своих — всегда).
    pub fn sees(&self, world: &World, team: u8, soldier: u32) -> bool {
        let s = &world.soldiers[soldier as usize];
        s.team == team || self.seen_until[team as usize & 1].get(soldier as usize).is_some_and(|&t| world.tick <= t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wind_sim::map::Map;
    use wind_sim::{SimConfig, Vec2};

    /// Синий №0 у стены; красные: №1 на виду, №2 за стеной, №3 далеко.
    fn world() -> World {
        let mut w = World::new(SimConfig { map_size: 128, soldiers_per_team: 3, capture_points: 1, ..Default::default() }, 1);
        let mut map = Map::new(128, 128);
        for y in 0..128 {
            map.set(60, y, 200);
        }
        w.map = map;
        let at = [(40.0, 40.0), (45.0, 45.0), (50.0, 50.0), (55.0, 55.0), (50.0, 30.0), (70.0, 40.0)];
        // Синие — 0, 1, 2; красные — 3, 4, 5.
        for (s, &(x, y)) in w.soldiers.iter_mut().zip(&at) {
            s.pos = Vec2::new(x, y);
        }
        w.soldiers[5].pos = Vec2::new(70.0, 40.0); // за стеной x = 60
        w.soldiers[3].pos = Vec2::new(10.0, 120.0); // далеко — дальше обзора
        w.soldiers[4].pos = Vec2::new(50.0, 30.0); // на виду
        w
    }

    #[test]
    fn enemies_behind_walls_or_far_are_not_seen() {
        let mut w = world();
        // Мир не делал шага — сетка солдат пуста; один шаг строит её (ИИ далеко не уйдёт).
        w.step();
        let mut v = Vision::default();
        v.update(&w);
        assert!(v.sees(&w, 0, 1), "свои видны всегда");
        assert!(v.sees(&w, 0, 4), "на виду");
        assert!(!v.sees(&w, 0, 5), "за стеной");
        assert!(!v.sees(&w, 0, 3), "дальше обзора");
    }

    #[test]
    fn seen_enemy_lingers_briefly() {
        let mut w = world();
        w.step();
        let mut v = Vision::default();
        v.update(&w);
        assert!(v.sees(&w, 0, 4));
        // Ушёл за стену — ещё полсекунды виден, потом нет.
        w.soldiers[4].pos = Vec2::new(80.0, 30.0);
        for _ in 0..LINGER {
            w.step();
            w.soldiers[4].pos = Vec2::new(80.0, 30.0);
            v.update(&w);
        }
        assert!(v.sees(&w, 0, 4));
        w.step();
        w.soldiers[4].pos = Vec2::new(80.0, 30.0);
        w.step();
        w.soldiers[4].pos = Vec2::new(80.0, 30.0);
        v.update(&w);
        assert!(!v.sees(&w, 0, 4));
    }
}
