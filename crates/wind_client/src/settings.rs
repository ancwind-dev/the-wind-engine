//! Настройки игрока: имя, громкость, экран, бой против ИИ, адрес сервера. Хранятся в
//! `%APPDATA%/The Wind Engine/settings.ron` (на Linux — `~/.config/the-wind-engine`),
//! а не в `data/`: это не контент игры, у каждого игрока свои.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub name: String,
    /// Громкость 0..1.
    pub volume: f32,
    /// Во весь экран (без рамки, в разрешении монитора) или в окне.
    pub fullscreen: bool,
    /// Размер окна в пикселях — когда игра в окне.
    pub resolution: (u32, u32),
    /// Бой против ИИ: солдат на команду и сторона случайной карты, м.
    pub soldiers_per_team: u32,
    pub map_size: u32,
    /// Карта из `data/maps/<имя>`; пусто — случайная.
    pub map_file: String,
    /// Последний сервер, к которому подключались.
    pub server: String,
    /// Пароль сервера — только на этот запуск: в файл настроек не пишем.
    #[serde(skip)]
    pub password: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            name: "Игрок".into(),
            volume: 0.8,
            fullscreen: true,
            resolution: (1600, 900),
            soldiers_per_team: 300,
            map_size: 512,
            map_file: String::new(),
            server: format!("127.0.0.1:{}", wind_net::DEFAULT_PORT),
            password: String::new(),
        }
    }
}

/// Пределы — те же, что у ползунков в меню.
pub const SOLDIERS: std::ops::RangeInclusive<u32> = 10..=1000;
pub const MAP_SIZE: std::ops::RangeInclusive<u32> = 128..=1024;

/// Размеры окна в списке, пиксели; больше монитора не предлагаем.
const RESOLUTIONS: [(u32, u32); 11] = [
    (1280, 720),
    (1366, 768),
    (1440, 900),
    (1600, 900),
    (1680, 1050),
    (1920, 1080),
    (1920, 1200),
    (2560, 1080),
    (2560, 1440),
    (3440, 1440),
    (3840, 2160),
];
const SMALLEST: (u32, u32) = (640, 360);
const LARGEST: (u32, u32) = (7680, 4320);

/// Какие размеры окна предложить: обычные, не больше монитора, плюс сам монитор и
/// `current` (его могли вписать в файл руками). По возрастанию.
pub fn resolutions(monitor: Option<(u32, u32)>, current: (u32, u32)) -> Vec<(u32, u32)> {
    let fits = |r: &(u32, u32)| monitor.is_none_or(|m| r.0 <= m.0 && r.1 <= m.1);
    let mut list: Vec<(u32, u32)> = RESOLUTIONS.iter().copied().filter(fits).chain(monitor).chain([current]).collect();
    list.sort_unstable();
    list.dedup();
    list
}

impl Settings {
    /// Где лежит файл настроек.
    pub fn path() -> PathBuf {
        let base = std::env::var_os("APPDATA")
            .map(|d| PathBuf::from(d).join("The Wind Engine"))
            .or_else(|| std::env::var_os("XDG_CONFIG_HOME").map(|d| PathBuf::from(d).join("the-wind-engine")))
            .or_else(|| std::env::var_os("HOME").map(|d| PathBuf::from(d).join(".config/the-wind-engine")))
            .unwrap_or_else(|| PathBuf::from("."));
        base.join("settings.ron")
    }

    /// Читает настройки; нет файла — `defaults`. Испорченный файл не мешает играть:
    /// берём `defaults` и пишем, что не так.
    pub fn load(path: &Path, defaults: Settings) -> (Settings, Option<String>) {
        if !path.exists() {
            return (defaults, None);
        }
        match wind_sim::config::read_text(path).and_then(|text| ron::from_str::<Settings>(&text).map_err(|e| e.to_string())) {
            Ok(s) => (s.clamped(), None),
            Err(e) => (defaults, Some(format!("{}: {e}; настройки по умолчанию", path.display()))),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default()).map_err(|e| e.to_string())?;
        std::fs::write(path, format!("// Настройки The Wind Engine. Меняются в игре: Esc → Настройки.\n{text}\n")).map_err(|e| e.to_string())
    }

    /// Числа — в пределах, которые умеет игра (файл могли поправить руками).
    pub fn clamped(mut self) -> Settings {
        self.volume = if self.volume.is_finite() { self.volume.clamp(0.0, 1.0) } else { 0.8 };
        self.soldiers_per_team = self.soldiers_per_team.clamp(*SOLDIERS.start(), *SOLDIERS.end());
        self.map_size = self.map_size.clamp(*MAP_SIZE.start(), *MAP_SIZE.end());
        self.resolution = (self.resolution.0.clamp(SMALLEST.0, LARGEST.0), self.resolution.1.clamp(SMALLEST.1, LARGEST.1));
        if self.name.trim().is_empty() {
            self.name = Settings::default().name;
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("wind-settings-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir.join("settings.ron")
    }

    #[test]
    fn saved_settings_come_back() {
        let path = temp("roundtrip");
        let s = Settings {
            name: "Артём".into(),
            volume: 0.35,
            fullscreen: false,
            resolution: (1366, 768),
            soldiers_per_team: 120,
            map_size: 256,
            map_file: "пример".into(),
            server: "10.0.0.2:7777".into(),
            password: String::new(),
        };
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path, Settings::default()), (s.clone(), None));
        // Пароль сервера в файл не попадает.
        let with_password = Settings { password: "тайна".into(), ..s.clone() };
        with_password.save(&path).unwrap();
        assert!(!std::fs::read_to_string(&path).unwrap().contains("тайна"));
        assert_eq!(Settings::load(&path, Settings::default()).0.password, "");
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn missing_partial_and_broken_files() {
        let path = temp("partial");
        // Нет файла — значения по умолчанию, без ошибки.
        let defaults = Settings { soldiers_per_team: 77, ..Default::default() };
        assert_eq!(Settings::load(&path, defaults.clone()), (defaults.clone(), None));
        // Только часть полей и числа за пределами — остальное по умолчанию, числа в пределах.
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "(volume: 7.0, soldiers_per_team: 5, resolution: (100, 99999))").unwrap();
        let (s, err) = Settings::load(&path, defaults.clone());
        assert!(err.is_none());
        assert_eq!((s.volume, s.soldiers_per_team, s.map_size), (1.0, 10, 512));
        assert_eq!(s.resolution, (640, 4320));
        // Старый файл без экрана — во весь экран.
        assert!(s.fullscreen);
        // Испорченный файл — значения по умолчанию и понятная ошибка.
        std::fs::write(&path, "(volume: ").unwrap();
        let (s, err) = Settings::load(&path, defaults.clone());
        assert_eq!(s, defaults);
        assert!(err.is_some());
        let _ = std::fs::remove_dir_all(path.parent().unwrap());
    }

    #[test]
    fn resolutions_fit_the_monitor() {
        let list = resolutions(Some((1920, 1080)), (1600, 900));
        assert_eq!(list.first(), Some(&(1280, 720)));
        assert_eq!(list.last(), Some(&(1920, 1080)));
        assert!(!list.contains(&(2560, 1440)), "больше монитора не предлагаем");
        // Свой размер из файла и необычный монитор — тоже в списке, без повторов.
        let list = resolutions(Some((2560, 1600)), (1000, 700));
        assert!(list.contains(&(1000, 700)) && list.contains(&(2560, 1600)));
        assert_eq!(list.iter().filter(|r| **r == (1920, 1080)).count(), 1);
    }
}
