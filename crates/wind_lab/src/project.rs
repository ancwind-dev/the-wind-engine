//! Проект — папка игры на движке: файл `wind.project` и папка данных (`data/`).
//!
//! Движок открывает проект, а не «папку, из которой запустили»: все редакторы берут
//! карты, модели и `sim.ron` из данных проекта. Наша игра — тоже проект (корень репозитория).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const FILE: &str = "wind.project";

/// Содержимое `wind.project`.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Manifest {
    pub name: String,
    pub description: String,
    /// Папка данных относительно корня проекта.
    pub data: PathBuf,
}

impl Default for Manifest {
    fn default() -> Self {
        Manifest { name: String::new(), description: String::new(), data: PathBuf::from("data") }
    }
}

#[derive(Clone, Debug)]
pub struct Project {
    pub root: PathBuf,
    pub manifest: Manifest,
}

/// С чего начать новый проект.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Template {
    /// Только параметры боя (`sim.ron`): процедурная карта, встроенные модели.
    Empty,
    /// Копия данных нашей игры: карты, солдаты, оружие, предметы, трава.
    Game,
}

/// Почему не получилось создать проект — текст для человека переводит окно проектов.
#[derive(Debug, PartialEq)]
pub enum CreateError {
    NoName,
    BadName,
    NotEmpty(PathBuf),
    Io(String),
}

impl Project {
    pub fn open(root: &Path) -> Result<Project, String> {
        let file = root.join(FILE);
        let text = std::fs::read_to_string(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        let mut manifest: Manifest =
            ron::from_str(text.trim_start_matches('\u{feff}')).map_err(|e| format!("{}: {e}", file.display()))?;
        if manifest.name.trim().is_empty() {
            manifest.name = root.file_name().map_or_else(|| "?".into(), |n| n.to_string_lossy().into_owned());
        }
        Ok(Project { root: crate::settings::normalize(root), manifest })
    }

    pub fn name(&self) -> &str {
        &self.manifest.name
    }

    pub fn data_dir(&self) -> PathBuf {
        self.root.join(&self.manifest.data)
    }

    pub fn config_path(&self) -> PathBuf {
        self.data_dir().join("sim.ron")
    }

    pub fn maps_dir(&self) -> PathBuf {
        self.data_dir().join("maps")
    }

    /// Названия карт проекта (папки в `data/maps`), по алфавиту.
    pub fn maps(&self) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(self.maps_dir())
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().join("map.ron").exists())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// Сколько файлов с таким расширением в папке данных (с подпапками) — для сводки на главном экране.
    pub fn count_files(&self, sub: &str, ext: &str) -> usize {
        fn walk(dir: &Path, ext: &str) -> usize {
            std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| {
                    let p = e.path();
                    if p.is_dir() {
                        walk(&p, ext)
                    } else {
                        usize::from(p.extension().is_some_and(|x| x.eq_ignore_ascii_case(ext)))
                    }
                })
                .sum()
        }
        walk(&self.data_dir().join(sub), ext)
    }

    /// Новый проект в `parent/<name>` из шаблона; данные шаблона берём из `sample`.
    pub fn create(parent: &Path, name: &str, template: Template, sample: &Project) -> Result<Project, CreateError> {
        let io = |path: &Path, e: std::io::Error| CreateError::Io(format!("{}: {e}", path.display()));
        let name = name.trim();
        if name.is_empty() {
            return Err(CreateError::NoName);
        }
        // Знаки, которые Windows не разрешает в именах папок.
        if name.chars().any(|c| matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')) || name.ends_with('.') {
            return Err(CreateError::BadName);
        }
        let root = parent.join(name);
        if root.exists() && std::fs::read_dir(&root).map_err(|e| io(&root, e))?.next().is_some() {
            return Err(CreateError::NotEmpty(root));
        }
        let data = root.join("data");
        std::fs::create_dir_all(data.join("maps")).map_err(|e| io(&data, e))?;
        match template {
            Template::Empty => {
                std::fs::copy(sample.config_path(), data.join("sim.ron")).map_err(|e| io(&sample.config_path(), e))?;
            }
            Template::Game => copy_dir(&sample.data_dir(), &data).map_err(CreateError::Io)?,
        }
        let manifest = Manifest { name: name.to_owned(), description: String::new(), data: PathBuf::from("data") };
        let text = ron::ser::to_string_pretty(&manifest, ron::ser::PrettyConfig::default())
            .map_err(|e| CreateError::Io(e.to_string()))?;
        let header = "// Проект The Wind Engine. Данные игры — в папке data.\n";
        std::fs::write(root.join(FILE), format!("{header}{text}\n")).map_err(|e| io(&root, e))?;
        Project::open(&root).map_err(CreateError::Io)
    }
}

/// Наша игра: корень репозитория с `wind.project`. Ищем от папки программы вверх,
/// а при запуске через cargo — от исходников.
pub fn sample() -> Option<Project> {
    let from_exe = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.ancestors().find(|d| d.join(FILE).exists()).map(Path::to_path_buf));
    let from_source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let root = from_exe.or_else(|| from_source.join(FILE).exists().then_some(from_source))?;
    Project::open(&root).ok()
}

/// Папка для новых проектов по умолчанию: `Документы/The Wind Engine`.
pub fn default_parent() -> PathBuf {
    let home = std::env::var_os("USERPROFILE").map_or_else(|| PathBuf::from("."), PathBuf::from);
    home.join("Documents").join("The Wind Engine")
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), String> {
    std::fs::create_dir_all(to).map_err(|e| format!("{}: {e}", to.display()))?;
    for entry in std::fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let (src, dst) = (entry.path(), to.join(entry.file_name()));
        if src.is_dir() {
            copy_dir(&src, &dst)?;
        } else {
            std::fs::copy(&src, &dst).map_err(|e| format!("{}: {e}", src.display()))?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn our_game_is_a_project_with_maps() {
        let game = sample().expect("wind.project в корне репозитория");
        assert!(game.config_path().exists());
        assert!(!game.maps().is_empty());
    }

    #[test]
    fn new_projects_open_and_run_from_both_templates() {
        let game = sample().unwrap();
        let parent = std::env::temp_dir().join("wind_lab_project_test");
        let _ = std::fs::remove_dir_all(&parent);
        for (name, template) in [("Пустой", Template::Empty), ("Копия", Template::Game)] {
            let p = Project::create(&parent, name, template, &game).unwrap();
            assert_eq!(p.name(), name);
            // Параметры боя читаются — значит, лаборатория откроет проект.
            wind_sim::SimConfig::load(&p.config_path()).unwrap();
            assert_eq!(p.maps().is_empty(), template == Template::Empty);
            let again = Project::create(&parent, name, template, &game);
            assert!(matches!(again, Err(CreateError::NotEmpty(_))), "в непустую папку не создаём");
        }
        assert_eq!(Project::create(&parent, "плохое/имя", Template::Empty, &game).unwrap_err(), CreateError::BadName);
        assert_eq!(Project::create(&parent, "  ", Template::Empty, &game).unwrap_err(), CreateError::NoName);
        let _ = std::fs::remove_dir_all(&parent);
    }
}
