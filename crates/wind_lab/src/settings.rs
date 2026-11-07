//! Настройки движка — общие для всех проектов: язык и недавние проекты.
//! Лежат рядом с настройками игры: `%APPDATA%/The Wind Engine/engine.ron`.

use crate::i18n::Lang;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Сколько недавних проектов помним.
const RECENT: usize = 12;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct EngineSettings {
    pub language: Lang,
    /// Масштаб интерфейса: 1 — как задумано, больше — крупнее (для больших мониторов).
    pub ui_scale: f32,
    /// Папки недавних проектов, последний открытый — первым.
    pub recent: Vec<PathBuf>,
}

impl Default for EngineSettings {
    fn default() -> Self {
        EngineSettings { language: Lang::Ru, ui_scale: 1.0, recent: Vec::new() }
    }
}

impl EngineSettings {
    pub fn path() -> PathBuf {
        let base = std::env::var_os("APPDATA").map_or_else(|| PathBuf::from("."), PathBuf::from);
        base.join("The Wind Engine").join("engine.ron")
    }

    /// Нет файла или он испорчен — настройки по умолчанию: запуск не должен падать из-за них.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path).ok().and_then(|text| ron::from_str(&text).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        }
        let text = ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::default()).map_err(|e| e.to_string())?;
        std::fs::write(path, text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Проект открыли — он первый в списке недавних.
    pub fn touch(&mut self, root: &Path) {
        let root = normalize(root);
        self.recent.retain(|p| normalize(p) != root);
        self.recent.insert(0, root);
        self.recent.truncate(RECENT);
    }

    pub fn forget(&mut self, root: &Path) {
        let root = normalize(root);
        self.recent.retain(|p| normalize(p) != root);
    }
}

/// Один и тот же проект, открытый по разным путям (`..`, `/` и `\`), — одна строка в списке.
pub fn normalize(path: &Path) -> PathBuf {
    let full = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    // На Windows canonicalize даёт `\?\C:\…` — такой путь людям показывать незачем.
    match full.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        Some(plain) => PathBuf::from(plain),
        None => full,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recent_projects_keep_order_without_duplicates() {
        let dir = std::env::temp_dir().join("wind_lab_settings_test");
        let (a, b) = (dir.join("a"), dir.join("b"));
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let mut s = EngineSettings::default();
        s.touch(&a);
        s.touch(&b);
        s.touch(&a.join("..").join("a"));
        assert_eq!(s.recent, vec![normalize(&a), normalize(&b)]);
        let file = dir.join("engine.ron");
        s.language = Lang::En;
        s.save(&file).unwrap();
        let back = EngineSettings::load(&file);
        assert_eq!(back.language, Lang::En);
        assert_eq!(back.recent, s.recent);
    }
}
