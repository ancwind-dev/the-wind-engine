//! Прицел и обзор, как в RWR. Числа сверены с файлами RWR (`default.ai`,
//! `default_base.character`, `base_primary.weapon`) и статьёй вики «Line of Sight, Vision»:
//!
//! - камера игрока выносится к курсору: в сторону прицела видно примерно на 80 % дальше
//!   (`view_center`); врагов на своём экране игрок видит и за стенами — тумана войны
//!   у самого игрока нет, а ИИ видит только по прямой;
//! - ИИ видит ближе, чем игрок: 32 м (у RWR — 30–35). Цель заметнее, когда бежит, и
//!   незаметнее, когда стоит или лежит; стреляющего видно дальше — по вспышкам;
//! - заметив врага, ИИ открывает огонь не сразу (0,3–0,6 с) и стреляет очередями
//!   с паузами;
//! - точность зависит от движения: на бегу разброс почти втрое больше, чем стоя.

use crate::world::Soldier;
use glam::Vec2;
use serde::Deserialize;

/// Насколько камера игрока выносится к прицелу: центр кадра — на этой доле пути
/// от солдата к прицелу. Курсор на краю экрана — камера сдвигается, курсор уходит
/// дальше; так сходится к обзору на `VIEW_LEAD / (1 − VIEW_LEAD)` ≈ 80 % дальше, чем
/// без выноса, и в стороны, и вверх-вниз экрана.
pub const VIEW_LEAD: f32 = 0.45;
/// Что видно на экране игрока — круг такого радиуса вокруг центра кадра, м
/// (камера RWR: 36 м, 45°; по ширине кадра — около ±26 м).
pub const SCREEN_RADIUS: f32 = 30.0;
/// Дальше этого от солдата прицел камеру не тянет, м.
const MAX_LEAD: f32 = 60.0;

/// Центр кадра игрока, который стоит в `pos` и целится в `aim`.
pub fn view_center(pos: Vec2, aim: Vec2) -> Vec2 {
    pos + (aim - pos).clamp_length_max(MAX_LEAD) * VIEW_LEAD
}

/// Виден ли `at` на экране игрока (с запасом: экран — круг).
pub fn on_screen(pos: Vec2, aim: Vec2, at: Vec2) -> bool {
    view_center(pos, aim).distance_squared(at) < SCREEN_RADIUS * SCREEN_RADIUS
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct SightSpec {
    /// Во сколько раз от дальности обзора ИИ видна цель: бежит, идёт, стоит, лежит.
    pub detect_running: f32,
    pub detect_walking: f32,
    pub detect_still: f32,
    pub detect_prone: f32,
    /// Стрелявшего в последние `muzzle_time` с видно в `muzzle_factor` раз дальше,
    /// но не дальше `muzzle_range` м.
    pub muzzle_factor: f32,
    pub muzzle_range: f32,
    pub muzzle_time: f32,
    /// Через сколько секунд ИИ открывает огонь по новой цели — случайно в этих пределах.
    pub reaction_min: f32,
    pub reaction_max: f32,
    /// Очередь ИИ и пауза после неё, с.
    pub burst_min: f32,
    pub burst_max: f32,
    pub pause_min: f32,
    pub pause_max: f32,
    /// Точность на бегу, шагом, стоя и лёжа; разброс — во столько раз больше, во сколько
    /// точность меньше, чем стоя.
    pub accuracy_running: f32,
    pub accuracy_walking: f32,
    pub accuracy_standing: f32,
    pub accuracy_prone: f32,
}

impl Default for SightSpec {
    fn default() -> Self {
        SightSpec {
            detect_running: 1.0,
            detect_walking: 0.95,
            detect_still: 0.88,
            detect_prone: 0.6,
            muzzle_factor: 1.7,
            muzzle_range: 40.0,
            muzzle_time: 1.0,
            reaction_min: 0.3,
            reaction_max: 0.6,
            burst_min: 0.1,
            burst_max: 0.5,
            pause_min: 0.6,
            pause_max: 1.8,
            accuracy_running: 0.3,
            accuracy_walking: 0.675,
            accuracy_standing: 0.85,
            accuracy_prone: 0.95,
        }
    }
}

/// Скорость шага и бега, м/с: между ними точность и заметность меняются плавно.
const WALK: f32 = 2.0;
const RUN: f32 = 4.5;

impl SightSpec {
    /// Во сколько раз от дальности обзора видна цель, которая движется со скоростью
    /// `speed` (лежит — `prone`).
    pub fn detectability(&self, speed: f32, prone: bool) -> f32 {
        if prone {
            return self.detect_prone;
        }
        by_speed(speed, self.detect_still, self.detect_walking, self.detect_running)
    }

    /// С какого расстояния ИИ с обзором `vision` замечает солдата `s`.
    pub fn sight_range(&self, vision: f32, s: &Soldier) -> f32 {
        let range = vision * self.detectability(s.vel.length(), s.prone);
        if s.shot_ago < self.muzzle_time {
            range.max((vision * self.muzzle_factor).min(self.muzzle_range))
        } else {
            range
        }
    }

    /// Во сколько раз разброс больше, чем стоя: на бегу — почти втрое.
    pub fn spread_factor(&self, speed: f32, prone: bool) -> f32 {
        let accuracy = if prone { self.accuracy_prone } else { by_speed(speed, self.accuracy_standing, self.accuracy_walking, self.accuracy_running) };
        self.accuracy_standing / accuracy.max(0.05)
    }
}

/// Плавно: стоит → идёт (до `WALK` м/с) → бежит (до `RUN`).
fn by_speed(speed: f32, still: f32, walking: f32, running: f32) -> f32 {
    if speed <= WALK {
        still + (walking - still) * (speed / WALK).clamp(0.0, 1.0)
    } else {
        walking + (running - walking) * ((speed - WALK) / (RUN - WALK)).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_is_seen_farther_and_shoots_wider() {
        let s = SightSpec::default();
        assert!(s.detectability(4.5, false) > s.detectability(2.0, false));
        assert!(s.detectability(2.0, false) > s.detectability(0.0, false));
        assert!(s.detectability(0.0, true) < s.detectability(0.0, false));
        assert_eq!(s.spread_factor(0.0, false), 1.0);
        assert!((s.spread_factor(4.5, false) - 0.85 / 0.3).abs() < 1e-4, "на бегу — почти втрое");
        assert!(s.spread_factor(0.0, true) < 1.0, "лёжа точнее");
    }

    #[test]
    fn view_reaches_about_80_percent_farther_toward_the_cursor() {
        // Курсор на краю экрана (26 м от центра кадра): центр кадра сходится туда, где
        // курсор тянет камеру ровно на свою долю.
        let pos = Vec2::ZERO;
        let mut center = pos;
        for _ in 0..50 {
            let aim = center + Vec2::X * 26.0;
            center = view_center(pos, aim);
        }
        let reach = center.x + 26.0;
        assert!((reach / 26.0 - 1.8).abs() < 0.05, "в сторону курсора видно на {:.0} % дальше", (reach / 26.0 - 1.0) * 100.0);
        assert!(on_screen(pos, Vec2::X * 40.0, Vec2::X * 45.0));
        assert!(!on_screen(pos, Vec2::X * 40.0, -Vec2::X * 20.0), "за спиной — уже за краем");
    }
}
