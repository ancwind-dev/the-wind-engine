//! Параметры матча, солдат и оружия. Читаются из `data/sim.ron`.
//!
//! Все поля имеют значения по умолчанию: в файле можно указать только то,
//! что хочется поменять.

use crate::arsenal::{Arsenal, WeaponSpec};
use crate::round::RoundSpec;
use crate::squad::SquadSpec;
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
    /// Оружие на всех — если рядом нет `weapons.ron` и `factions.ron`.
    pub weapon: WeaponSpec,
    pub grenade: GrenadeSpec,
    pub capture: CaptureSpec,
    /// Правила раунда: сколько держать точки, длина раунда, перерыв.
    pub round: RoundSpec,
    /// Отряды: сколько солдат, как плотно идут строем.
    pub squad: SquadSpec,
    /// Оружие и классы: из `weapons.ron` и `factions.ron` рядом с этим файлом,
    /// а без них — одно оружие `weapon` на всех.
    #[serde(skip)]
    pub arsenal: Arsenal,
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
    /// Читает `sim.ron`, а оружие и классы — из `weapons.ron` и `factions.ron` рядом.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = read_text(path)?;
        let mut config: SimConfig = ron::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        let dir = path.parent().unwrap_or(Path::new("."));
        config.arsenal = if Arsenal::exists(dir) { Arsenal::load(dir)? } else { Arsenal::single(config.weapon.clone()) };
        Ok(config)
    }

    pub fn dt(&self) -> f32 {
        1.0 / self.tick_rate.max(1) as f32
    }
}

/// Текст файла данных. Блокнот Windows (и PowerShell) ставят в начало невидимую
/// метку BOM — без её удаления файл не разбирается, и игра молча берёт настройки
/// по умолчанию.
pub fn read_text(path: &Path) -> Result<String, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(text.strip_prefix('\u{feff}').map(str::to_owned).unwrap_or(text))
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
            round: RoundSpec::default(),
            squad: SquadSpec::default(),
            arsenal: Arsenal::default(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_with_bom_is_read() {
        let dir = std::env::temp_dir().join(format!("wind_sim_bom_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("sim.ron");
        std::fs::write(&path, "\u{feff}(tick_rate: 30, round: (time_limit: 20.0))").unwrap();
        let c = SimConfig::load(&path).unwrap();
        assert_eq!(c.tick_rate, 30);
        assert_eq!(c.round.time_limit, 20.0);
        std::fs::remove_dir_all(&dir).ok();
    }
}
