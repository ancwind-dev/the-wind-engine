//! Предметы на карте: модели из `data/models/props/*.glb` (их делают в Blender).
//!
//! Папка перечитывается на лету: сохранил модель в Blender — она тут же поменялась в
//! игре и в редакторе. Имя модели — имя файла без `.glb`.

use crate::voxel::Mesh;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// Как часто заглядывать в папку.
const POLL: Duration = Duration::from_millis(500);

#[derive(Default)]
pub struct PropLibrary {
    dir: Option<PathBuf>,
    /// Имя модели → (когда файл изменён, радиус модели вокруг начала, м).
    models: HashMap<String, (SystemTime, f32)>,
    last_poll: Option<Instant>,
}

impl PropLibrary {
    pub fn new(dir: PathBuf) -> Self {
        PropLibrary { dir: Some(dir), ..Default::default() }
    }

    /// Радиус модели вокруг её начала, м — чтобы не рисовать дальние; `None` — модели
    /// нет (пока или вовсе).
    pub fn radius(&self, name: &str) -> Option<f32> {
        self.models.get(name).map(|m| m.1)
    }

    /// Заглянуть в папку (не чаще раза в полсекунды): новые и изменённые модели —
    /// (имя, меш); что не прочиталось — строками.
    pub fn poll(&mut self) -> (Vec<(String, Mesh)>, Vec<String>) {
        let (mut loaded, mut errors) = (Vec::new(), Vec::new());
        let Some(dir) = &self.dir else { return (loaded, errors) };
        if self.last_poll.is_some_and(|t| t.elapsed() < POLL) {
            return (loaded, errors);
        }
        self.last_poll = Some(Instant::now());
        for (name, path, modified) in list(dir) {
            if self.models.get(&name).is_some_and(|m| m.0 == modified) {
                continue;
            }
            // Файл могли поймать недописанным: когда допишут, время изменения сменится
            // и прочитаем снова.
            match std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| crate::gltf::load(&b)) {
                Ok(mesh) => {
                    let radius = mesh.vertices.iter().map(|v| (v.pos[0].powi(2) + v.pos[2].powi(2)).sqrt()).fold(0.0, f32::max);
                    self.models.insert(name.clone(), (modified, radius));
                    loaded.push((name, mesh));
                }
                Err(e) => {
                    self.models.insert(name.clone(), (modified, self.radius(&name).unwrap_or(0.0)));
                    errors.push(format!("модель {}: {e}", path.display()));
                }
            }
        }
        (loaded, errors)
    }
}

/// Модели в папке: (имя, путь, когда изменён) — по имени.
pub fn list(dir: &Path) -> Vec<(String, PathBuf, SystemTime)> {
    let mut out: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let name = path.file_stem()?.to_str()?.to_owned();
            (path.extension()?.to_str()?.eq_ignore_ascii_case("glb")).then_some(())?;
            Some((name, path.clone(), e.metadata().ok()?.modified().ok()?))
        })
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn models_load_and_reload_when_the_file_changes() {
        let dir = std::env::temp_dir().join(format!("wind-props-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let glb = crate::gltf::tests::triangle_glb();
        std::fs::write(dir.join("дом.glb"), &glb).unwrap();
        std::fs::write(dir.join("сломанный.glb"), b"not a glb").unwrap();
        std::fs::write(dir.join("заметка.txt"), b"-").unwrap();

        let mut lib = PropLibrary::new(dir.clone());
        let (loaded, errors) = lib.poll();
        assert_eq!(loaded.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>(), ["дом"]);
        assert_eq!(errors.len(), 1, "сломанный файл — строкой, не паникой");
        assert!(lib.radius("дом").is_some_and(|r| r > 11.0));
        // Сразу — не смотрим; через полсекунды без изменений — ничего нового.
        assert!(lib.poll().0.is_empty());
        std::thread::sleep(POLL + Duration::from_millis(50));
        assert!(lib.poll().0.is_empty());

        // Файл переписали (как при сохранении из Blender) — модель перечитана.
        std::thread::sleep(Duration::from_millis(20));
        std::fs::write(dir.join("дом.glb"), &glb).unwrap();
        let f = std::fs::File::options().write(true).open(dir.join("дом.glb")).unwrap();
        f.set_modified(SystemTime::now() + Duration::from_secs(1)).unwrap();
        drop(f);
        std::thread::sleep(POLL + Duration::from_millis(50));
        assert_eq!(lib.poll().0.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

