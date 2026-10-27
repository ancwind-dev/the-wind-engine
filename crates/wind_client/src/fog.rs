//! Туман войны: врага видно, только если его видит свой солдат или кто-то из своих
//! рядом — в пределах дальности обзора и по прямой, не сквозь стены. Считается
//! на клиенте (сервер присылает всех в радиусе интереса).

use std::collections::HashSet;
use wind_render::SoldierDraw;
use wind_sim::Vec2;
use wind_sim::map::Map;

/// Сколько своих солдат (ближайших) «смотрят» за команду.
const OBSERVERS: usize = 16;
/// Как часто пересчитывать, с: лучей сотни, каждый кадр незачем.
const PERIOD: f32 = 0.1;

#[derive(Default)]
pub struct Fog {
    visible: HashSet<u32>,
    timer: f32,
}

impl Fog {
    /// `own_team` — наша команда (`None` — наблюдаем, видно всех), `center` — свой
    /// солдат или центр камеры, если он убит.
    pub fn update(&mut self, dt: f32, map: &Map, vision: f32, own_team: Option<u8>, center: Vec2, soldiers: &[SoldierDraw]) {
        self.timer -= dt;
        if self.timer > 0.0 {
            return;
        }
        self.timer = PERIOD;
        self.visible.clear();
        let Some(team) = own_team else { return };
        let mut observers: Vec<Vec2> = soldiers.iter().filter(|s| s.team == team).map(|s| s.pos).collect();
        observers.sort_by(|a, b| a.distance_squared(center).total_cmp(&b.distance_squared(center)));
        observers.truncate(OBSERVERS);
        let v2 = vision * vision;
        for s in soldiers.iter().filter(|s| s.team != team) {
            let seen = observers.iter().any(|&o| o.distance_squared(s.pos) < v2 && map.line_of_sight(o, s.pos));
            if seen {
                self.visible.insert(s.id);
            }
        }
    }

    /// Видно ли солдата: свои — всегда, враги — если их кто-то видит.
    pub fn shows(&self, s: &SoldierDraw, own_team: Option<u8>) -> bool {
        own_team.is_none_or(|t| t == s.team) || self.visible.contains(&s.id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn soldier(id: u32, team: u8, x: f32, y: f32) -> SoldierDraw {
        SoldierDraw { id, pos: Vec2::new(x, y), facing: 0.0, team, look: None }
    }

    #[test]
    fn enemies_behind_walls_or_far_are_hidden() {
        let mut map = Map::new(64, 64);
        for y in 0..64 {
            map.set(30, y, 120);
        }
        let soldiers = [
            soldier(0, 0, 10.0, 20.0),
            // В открытую, в 15 м.
            soldier(1, 1, 25.0, 20.0),
            // За стеной.
            soldier(2, 1, 40.0, 20.0),
            // Далеко (дальше обзора), без стены.
            soldier(3, 1, 10.0, 63.5),
        ];
        let mut fog = Fog::default();
        fog.update(1.0, &map, 30.0, Some(0), soldiers[0].pos, &soldiers);
        let shown: Vec<u32> = soldiers.iter().filter(|s| fog.shows(s, Some(0))).map(|s| s.id).collect();
        assert_eq!(shown, vec![0, 1]);
        // Наблюдатель видит всех.
        assert!(soldiers.iter().all(|s| fog.shows(s, None)));
    }
}
