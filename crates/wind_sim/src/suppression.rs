//! Подавление огнём и раненые.
//!
//! - Каждая пуля рядом добавляет солдату «подавление» (0..1), оно спадает само.
//!   Подавленный хуже целится, а сильно подавленный **залегает**: почти не двигается,
//!   но и попасть в него труднее.
//! - Автоматчик, потерявший врага из виду, ещё несколько секунд стреляет туда, где
//!   враг был, — **огонь на подавление**: тот, кто спрятался за угол, там и сидит.
//! - **Раненый** отходит туда, где своих больше и врага нет: к своей точке подальше
//!   от стрелка или на базу. Вне боя раны понемногу заживают.
//!
//! Всё это — только у солдат ИИ; игрок управляет собой сам (лечится и он).

use crate::world::CapturePoint;
use glam::Vec2;
use serde::Deserialize;

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct SuppressionSpec {
    /// Сколько подавления добавляет пуля рядом и попадание.
    pub per_near_miss: f32,
    pub per_hit: f32,
    /// Сколько спадает за секунду.
    pub decay: f32,
    /// Во сколько раз растёт разброс при полном подавлении (1 — вдвое).
    pub spread_penalty: f32,
    /// Залегает при подавлении выше `prone_at`, встаёт — ниже `stand_at`.
    pub prone_at: f32,
    pub stand_at: f32,
    /// Лежачий — меньшая мишень: радиус для попаданий, доля обычного.
    pub prone_target: f32,
    /// Скорость ползком, доля обычной.
    pub crawl: f32,
    /// Сколько секунд стрелять туда, где враг был (0 — не стрелять на подавление).
    pub suppress_memory: f32,
    /// Здоровье, ниже которого раненый отходит, и выше которого возвращается в бой.
    pub retreat_below: f32,
    pub return_above: f32,
    /// Заживление: столько здоровья в секунду, если `heal_delay` секунд нет боя.
    pub heal_rate: f32,
    pub heal_delay: f32,
}

impl Default for SuppressionSpec {
    fn default() -> Self {
        SuppressionSpec {
            per_near_miss: 0.12,
            per_hit: 0.35,
            decay: 0.25,
            spread_penalty: 1.5,
            prone_at: 0.6,
            stand_at: 0.25,
            prone_target: 0.55,
            crawl: 0.25,
            suppress_memory: 3.0,
            retreat_below: 0.65,
            return_above: 0.9,
            heal_rate: 0.03,
            heal_delay: 5.0,
        }
    }
}

/// Куда отходить раненому — точка захвата.
pub const RETREAT_TO_BASE: u32 = u32::MAX;

/// Куда отходить из `pos` от врага в `threat`: ближайшая своя точка, которая дальше
/// от врага, чем мы сейчас (и не рядом с ним); если такой нет — на базу.
pub fn retreat_point(points: &[CapturePoint], team: u8, pos: Vec2, threat: Option<Vec2>) -> u32 {
    let away = |p: Vec2| threat.is_none_or(|t| p.distance(t) > pos.distance(t) + 5.0 && p.distance(t) > 25.0);
    points
        .iter()
        .enumerate()
        .filter(|(_, p)| p.owner == Some(team) && away(p.pos))
        .min_by(|a, b| a.1.pos.distance_squared(pos).total_cmp(&b.1.pos.distance_squared(pos)))
        .map_or(RETREAT_TO_BASE, |(i, _)| i as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x: f32, owner: Option<u8>) -> CapturePoint {
        CapturePoint { pos: Vec2::new(x, 0.0), owner, progress: 0.0 }
    }

    #[test]
    fn wounded_retreat_away_from_the_enemy() {
        // Свои точки на 20 и 80, чужая на 60; мы на 50, враг на 70.
        let points = [point(20.0, Some(0)), point(60.0, Some(1)), point(80.0, Some(0))];
        let me = Vec2::new(50.0, 0.0);
        assert_eq!(retreat_point(&points, 0, me, Some(Vec2::new(70.0, 0.0))), 0, "к своей точке позади, а не к той, что за врагом");
        // Своих точек позади нет — на базу.
        assert_eq!(retreat_point(&points, 0, me, Some(Vec2::new(10.0, 0.0))), 2);
        assert_eq!(retreat_point(&points, 1, me, Some(Vec2::new(70.0, 0.0))), RETREAT_TO_BASE);
    }
}
