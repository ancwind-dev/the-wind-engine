//! Параметры матча, солдат и оружия. Читаются из `data/sim.ron`.
//!
//! Все поля имеют значения по умолчанию: в файле можно указать только то,
//! что хочется поменять.

use serde::Deserialize;
use std::path::Path;

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct SimConfig {
    /// Сторона карты в метрах (= клетках). Меняется только перезапуском матча.
    pub map_size: u32,
    /// Солдат в каждой команде. Меняется только перезапуском матча.
    pub soldiers_per_team: u32,
    /// Точек захвата. Меняется только перезапуском матча.
    pub capture_points: u32,
    /// Тиков симуляции в секунду.
    pub tick_rate: u32,
    /// ИИ солдата думает раз в столько тиков. Солдаты думают вразнобой.
    pub think_interval: u32,
    /// Вероятность, что солдат на захваченной точке останется её защищать
    /// (проверяется при каждом «обдумывании»).
    pub defend_probability: f32,
    pub soldier: SoldierSpec,
    pub weapon: WeaponSpec,
    pub grenade: GrenadeSpec,
    pub capture: CaptureSpec,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct SoldierSpec {
    /// Скорость бега, м/с.
    pub speed: f32,
    pub radius: f32,
    /// Дальность обзора, м.
    pub vision_range: f32,
    /// Сколько ближайших врагов проверять на прямую видимость за одно «обдумывание».
    pub vision_candidates: u32,
    pub respawn_time: f32,
}

/// Названия полей повторяют `.weapon`-файлы RWR, где это возможно.
#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct WeaponSpec {
    /// Пауза между выстрелами, с.
    pub retrigger_time: f32,
    pub magazine_size: u32,
    pub reload_time: f32,
    /// Скорость пули, м/с.
    pub projectile_speed: f32,
    /// Дальность стрельбы, м.
    pub range: f32,
    /// Разброс в градусах.
    pub spread_deg: f32,
    /// Вероятность, что попадание убьёт сразу.
    pub kill_probability: f32,
    /// Урон, если попадание не убило (здоровье солдата = 1.0).
    pub wound_damage: f32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct GrenadeSpec {
    /// Гранат при появлении.
    pub count: u32,
    pub throw_speed: f32,
    /// Время до взрыва, с.
    pub fuse: f32,
    pub blast_radius: f32,
    /// Вероятность убить в центре взрыва; к краю падает до нуля.
    pub kill_probability: f32,
    /// Урон по стенам в центре взрыва (прочность стены 1..254).
    pub wall_damage: f32,
    pub wall_damage_radius: f32,
    pub min_distance: f32,
    pub max_distance: f32,
    /// Бросать, только если рядом с целью столько врагов или больше.
    pub min_enemies: u32,
    /// Минимальная пауза между бросками одного солдата, с.
    pub interval: f32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct CaptureSpec {
    pub radius: f32,
    /// Секунд, чтобы один солдат захватил нейтральную точку. Больше солдат — быстрее.
    pub time: f32,
}

impl SimConfig {
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        ron::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn dt(&self) -> f32 {
        1.0 / self.tick_rate.max(1) as f32
    }
}

impl Default for SimConfig {
    fn default() -> Self {
        SimConfig {
            map_size: 512,
            soldiers_per_team: 300,
            capture_points: 5,
            tick_rate: 20,
            think_interval: 5,
            defend_probability: 0.97,
            soldier: SoldierSpec::default(),
            weapon: WeaponSpec::default(),
            grenade: GrenadeSpec::default(),
            capture: CaptureSpec::default(),
        }
    }
}

impl Default for SoldierSpec {
    fn default() -> Self {
        SoldierSpec {
            speed: 4.5,
            radius: 0.35,
            vision_range: 55.0,
            vision_candidates: 4,
            respawn_time: 6.0,
        }
    }
}

impl Default for WeaponSpec {
    fn default() -> Self {
        WeaponSpec {
            retrigger_time: 0.15,
            magazine_size: 30,
            reload_time: 2.5,
            projectile_speed: 90.0,
            range: 50.0,
            spread_deg: 4.0,
            kill_probability: 0.6,
            wound_damage: 0.4,
        }
    }
}

impl Default for GrenadeSpec {
    fn default() -> Self {
        GrenadeSpec {
            count: 2,
            throw_speed: 14.0,
            fuse: 2.0,
            blast_radius: 6.0,
            kill_probability: 0.9,
            wall_damage: 160.0,
            wall_damage_radius: 3.0,
            min_distance: 8.0,
            max_distance: 30.0,
            min_enemies: 3,
            interval: 15.0,
        }
    }
}

impl Default for CaptureSpec {
    fn default() -> Self {
        CaptureSpec { radius: 10.0, time: 12.0 }
    }
}
