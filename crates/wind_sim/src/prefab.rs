//! Готовые здания: дома, сараи, дворы, руины — шаблоны из клеток. Ими строит
//! генератор карт, их же вставляет редактор.
//!
//! Шаблон записан текстом, по строке на ряд клеток — его можно поправить в любом
//! редакторе (свои здания лежат в `data/prefabs.ron`):
//!
//! | знак | клетка |
//! |---|---|
//! | `#` | стена дома (прочность 120) |
//! | `%` | бетон (200) |
//! | `=` | забор (40) |
//! | `@` | скала (не ломается) |
//! | `.` | пол: стены нет, земля — бетон |
//! | `_` | дорога: стены нет, земля — асфальт |
//! | `-` | пусто: стены нет, земля как была |
//! | пробел | ничего не трогать |

use crate::map::{EMPTY, INDESTRUCTIBLE, Map};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const HOUSE: u8 = 120;
pub const CONCRETE: u8 = 200;
pub const FENCE: u8 = 40;
/// Покрытие земли (номера из `GROUND_NAMES`).
pub const ASPHALT: u8 = 4;
pub const FLOOR: u8 = 5;

/// Клетка шаблона: что поставить в стену и в землю (`None` — не трогать).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    pub wall: Option<u8>,
    pub ground: Option<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Prefab {
    pub name: String,
    pub width: i32,
    pub height: i32,
    cells: Vec<Cell>,
}

/// Так шаблон лежит в файле.
#[derive(Serialize, Deserialize)]
struct Text {
    name: String,
    rows: Vec<String>,
}

fn cell_of(c: char) -> Result<Cell, String> {
    let (wall, ground) = match c {
        '#' => (Some(HOUSE), None),
        '%' => (Some(CONCRETE), None),
        '=' => (Some(FENCE), None),
        '@' => (Some(INDESTRUCTIBLE), None),
        '.' => (Some(EMPTY), Some(FLOOR)),
        '_' => (Some(EMPTY), Some(ASPHALT)),
        '-' => (Some(EMPTY), None),
        ' ' => (None, None),
        other => return Err(format!("непонятный знак «{other}»")),
    };
    Ok(Cell { wall, ground })
}

fn char_of(c: Cell) -> char {
    match (c.wall, c.ground) {
        (None, _) => ' ',
        (Some(EMPTY), Some(ASPHALT)) => '_',
        (Some(EMPTY), Some(_)) => '.',
        (Some(EMPTY), None) => '-',
        (Some(INDESTRUCTIBLE), _) => '@',
        (Some(v), _) if v < 80 => '=',
        (Some(v), _) if v < 160 => '#',
        (Some(_), _) => '%',
    }
}

impl Prefab {
    /// Шаблон из строк (все одной длины).
    pub fn from_rows(name: &str, rows: &[&str]) -> Result<Prefab, String> {
        let height = rows.len() as i32;
        let width = rows.first().map_or(0, |r| r.chars().count()) as i32;
        if width == 0 || height == 0 {
            return Err(format!("здание «{name}»: пустое"));
        }
        let mut cells = Vec::with_capacity((width * height) as usize);
        for (y, row) in rows.iter().enumerate() {
            if row.chars().count() as i32 != width {
                return Err(format!("здание «{name}»: ряд {} другой длины", y + 1));
            }
            for c in row.chars() {
                cells.push(cell_of(c).map_err(|e| format!("здание «{name}», ряд {}: {e}", y + 1))?);
            }
        }
        Ok(Prefab { name: name.to_owned(), width, height, cells })
    }

    /// Шаблон из клеток (копия куска карты в редакторе).
    pub fn from_cells(name: &str, width: i32, height: i32, cells: Vec<Cell>) -> Prefab {
        assert_eq!(cells.len(), (width * height) as usize);
        Prefab { name: name.to_owned(), width, height, cells }
    }

    pub fn cell(&self, x: i32, y: i32) -> Cell {
        self.cells[(y * self.width + x) as usize]
    }

    pub fn rows(&self) -> Vec<String> {
        (0..self.height).map(|y| (0..self.width).map(|x| char_of(self.cell(x, y))).collect()).collect()
    }

    /// Повёрнутый на `quarters` четвертей по часовой стрелке.
    pub fn rotated(&self, quarters: u8) -> Prefab {
        let mut p = self.clone();
        for _ in 0..quarters % 4 {
            let (w, h) = (p.width, p.height);
            let mut cells = Vec::with_capacity(p.cells.len());
            // Новый ряд y — это старый столбец y, снизу вверх.
            for y in 0..w {
                for x in 0..h {
                    cells.push(p.cell(y, h - 1 - x));
                }
            }
            p = Prefab { name: p.name, width: h, height: w, cells };
        }
        p
    }

    /// Ставит здание на карту левым верхним углом в клетку (`x0`, `y0`).
    pub fn stamp(&self, map: &mut Map, x0: i32, y0: i32) {
        for y in 0..self.height {
            for x in 0..self.width {
                let c = self.cell(x, y);
                let Some(i) = map.index(x0 + x, y0 + y) else { continue };
                if let Some(w) = c.wall {
                    map.solid[i] = w;
                }
                if let Some(g) = c.ground {
                    map.ground[i] = g;
                }
            }
        }
    }
}

/// Встроенные здания — ими строит генератор, их всегда предлагает редактор.
pub fn builtin() -> Vec<Prefab> {
    let list: [(&str, &[&str]); 10] = [
        ("Дом малый", &["###..###", "#......#", "#......#", "#.......", "#.......", "#......#", "########"]),
        (
            "Дом в две комнаты",
            &[
                "#####..#####",
                "#....#.....#",
                "#....#.....#",
                "#..........#",
                ".....#.....#",
                ".....#.....#",
                "#....#.....#",
                "#....#.....#",
                "############",
            ],
        ),
        (
            "Дом угловой",
            &[
                "######      ",
                "#....#      ",
                "#.....      ",
                "#.....      ",
                "#....#######",
                "#..........#",
                "#..........#",
                "#...........",
                "#...........",
                "############",
            ],
        ),
        ("Руины", &["##.#..####", "#........#", "..........", "#.........", "#........#", "#........#", "###...#.##"]),
        ("Сарай", &["#####", "#...#", "#....", "#....", "#####"]),
        ("Бункер", &["%%%%%%%", "%.....%", "%......", "%......", "%.....%", "%%%%%%%"]),
        (
            "Двор с домом",
            &[
                "==============",
                "=------------=",
                "=-#####------=",
                "=-#...#------=",
                "=-#....-------",
                "=-#....-------",
                "=-#####------=",
                "=------------=",
                "==============",
            ],
        ),
        (
            "Магазин",
            &[
                "%%%%%%..%%%%%%%%",
                "%..............%",
                "%..............%",
                "%....%%%%%.....%",
                "%..............%",
                "...............%",
                "...............%",
                "%..............%",
                "%..............%",
                "%%%%%%%%%%%%%%%%",
            ],
        ),
        ("Забор", &["=========="]),
        ("Бетонная стенка", &["%%%%%%"]),
    ];
    list.iter().map(|(name, rows)| Prefab::from_rows(name, rows).expect("встроенные здания записаны верно")).collect()
}

/// Свои здания из `data/prefabs.ron`; нет файла — пустой список.
pub fn load(path: &Path) -> Result<Vec<Prefab>, String> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text = crate::config::read_text(path)?;
    let list: Vec<Text> = ron::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    list.iter()
        .map(|t| Prefab::from_rows(&t.name, &t.rows.iter().map(String::as_str).collect::<Vec<_>>()))
        .collect()
}

pub fn save(path: &Path, prefabs: &[Prefab]) -> Result<(), String> {
    let list: Vec<Text> = prefabs.iter().map(|p| Text { name: p.name.clone(), rows: p.rows() }).collect();
    let text = ron::ser::to_string_pretty(&list, ron::ser::PrettyConfig::default()).map_err(|e| e.to_string())?;
    let header = "// Свои здания для редактора карт. Знаки: # дом, % бетон, = забор, @ скала,\n\
                  // . пол (бетон), _ дорога (асфальт), - пусто, пробел — не трогать.\n";
    std::fs::write(path, format!("{header}{text}\n")).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nav::{FlowField, UNREACHABLE};
    use glam::Vec2;

    /// Пол каждого здания достижим снаружи: двери есть и проходимы. И встроенные,
    /// и свои из `data/prefabs.ron`.
    #[test]
    fn every_builtin_has_a_way_in() {
        let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/prefabs.ron");
        let mine = load(&data).expect("data/prefabs.ron читается");
        assert!(!mine.is_empty());
        for p in builtin().into_iter().chain(mine) {
            for q in 0..4 {
                let p = p.rotated(q);
                let mut map = Map::new((p.width + 6) as u32, (p.height + 6) as u32);
                p.stamp(&mut map, 3, 3);
                let mut field = FlowField::new(Vec2::new(1.5, 1.5));
                field.compute(&map);
                for y in 0..p.height {
                    for x in 0..p.width {
                        if p.cell(x, y).ground == Some(FLOOR) {
                            let i = map.index(x + 3, y + 3).unwrap();
                            assert_ne!(field.dist[i], UNREACHABLE, "«{}» (поворот {q}): в клетку ({x}, {y}) не войти", p.name);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn rotation_and_text_round_trip() {
        for p in builtin() {
            assert_eq!(p.rotated(4), p);
            let r = p.rotated(1);
            assert_eq!((r.width, r.height), (p.height, p.width));
            // Угол сверху слева уходит вправо вверх.
            assert_eq!(r.cell(r.width - 1, 0), p.cell(0, 0));
            let rows = p.rows();
            let back = Prefab::from_rows(&p.name, &rows.iter().map(String::as_str).collect::<Vec<_>>()).unwrap();
            assert_eq!(back, p);
        }
    }

    #[test]
    fn user_prefabs_save_and_load() {
        let dir = std::env::temp_dir().join(format!("wind_prefabs_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("prefabs.ron");
        assert!(load(&path).unwrap().is_empty());
        let mine = vec![builtin()[0].rotated(1), Prefab::from_rows("Пост", &["%%%", "%.%", "% %"]).unwrap()];
        save(&path, &mine).unwrap();
        assert_eq!(load(&path).unwrap(), mine);
        std::fs::remove_dir_all(&dir).ok();
        assert!(Prefab::from_rows("кривое", &["##", "#"]).is_err());
        assert!(Prefab::from_rows("знак", &["#?"]).is_err());
    }
}
