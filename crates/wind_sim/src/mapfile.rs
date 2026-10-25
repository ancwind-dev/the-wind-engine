//! Карта в файлах: папка `data/maps/<имя>/`.
//!
//! - `map.ron` — название, размер, базы, точки захвата;
//! - `walls.png` — прочность стен (8 бит, 0 — пусто, 255 — скала);
//! - `height.png` — высота земли в углах клеток (16 бит, 1/256 м), на 1 больше по каждой стороне;
//! - `ground.png` — покрытие земли (8 бит, номер из `GROUND_NAMES`).
//!
//! Слои — обычные картинки, их можно посмотреть и поправить любым графическим редактором,
//! как карты RWR.

use crate::map::Map;
use crate::mapgen::{self, Layout};
use glam::Vec2;
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use std::path::Path;

#[derive(Serialize, Deserialize)]
struct Meta {
    name: String,
    width: u32,
    height: u32,
    bases: [(f32, f32); 2],
    capture_points: Vec<(f32, f32)>,
}

#[derive(Clone)]
pub struct MapFile {
    pub name: String,
    pub map: Map,
    pub layout: Layout,
}

impl MapFile {
    /// Процедурная карта — как раньше генерировал `World::new`.
    pub fn generated(name: &str, size: u32, capture_points: u32, seed: u64) -> Self {
        let (map, layout) = mapgen::generate(size, capture_points, seed);
        MapFile { name: name.to_owned(), map, layout }
    }

    pub fn load(dir: &Path) -> Result<Self, String> {
        let err = |what: &str, e: &dyn std::fmt::Display| format!("{}: {what}: {e}", dir.display());
        let text = std::fs::read_to_string(dir.join("map.ron")).map_err(|e| err("map.ron", &e))?;
        let meta: Meta = ron::from_str(&text).map_err(|e| err("map.ron", &e))?;
        let (w, h) = (meta.width, meta.height);
        let mut map = Map::new(w, h);
        map.solid = read_png(&dir.join("walls.png"), w, h, 8).map_err(|e| err("walls.png", &e))?;
        map.ground = read_png(&dir.join("ground.png"), w, h, 8).map_err(|e| err("ground.png", &e))?;
        let raw = read_png(&dir.join("height.png"), w + 1, h + 1, 16).map_err(|e| err("height.png", &e))?;
        map.heights = raw.as_chunks::<2>().0.iter().map(|b| u16::from_be_bytes(*b)).collect();
        let v = |(x, y): (f32, f32)| Vec2::new(x, y);
        Ok(MapFile {
            name: meta.name,
            map,
            layout: Layout {
                bases: [v(meta.bases[0]), v(meta.bases[1])],
                capture_points: meta.capture_points.into_iter().map(v).collect(),
            },
        })
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        let err = |what: &str, e: &dyn std::fmt::Display| format!("{}: {what}: {e}", dir.display());
        std::fs::create_dir_all(dir).map_err(|e| err("папка", &e))?;
        let (w, h) = (self.map.width, self.map.height);
        let p = |v: Vec2| (v.x, v.y);
        let meta = Meta {
            name: self.name.clone(),
            width: w,
            height: h,
            bases: [p(self.layout.bases[0]), p(self.layout.bases[1])],
            capture_points: self.layout.capture_points.iter().copied().map(p).collect(),
        };
        let text = ron::ser::to_string_pretty(&meta, ron::ser::PrettyConfig::default()).map_err(|e| err("map.ron", &e))?;
        std::fs::write(dir.join("map.ron"), text).map_err(|e| err("map.ron", &e))?;
        write_png(&dir.join("walls.png"), w, h, 8, &self.map.solid).map_err(|e| err("walls.png", &e))?;
        write_png(&dir.join("ground.png"), w, h, 8, &self.map.ground).map_err(|e| err("ground.png", &e))?;
        let heights: Vec<u8> = self.map.heights.iter().flat_map(|v| v.to_be_bytes()).collect();
        write_png(&dir.join("height.png"), w + 1, h + 1, 16, &heights).map_err(|e| err("height.png", &e))?;
        Ok(())
    }

    /// Отпечаток карты: если у клиента и сервера он разный — карты разные.
    pub fn fingerprint(&self) -> u64 {
        let mut h = Fnv::default();
        h.write(self.name.as_bytes());
        h.write(&self.map.width.to_le_bytes());
        h.write(&self.map.height.to_le_bytes());
        for p in self.layout.bases.iter().chain(&self.layout.capture_points) {
            h.write(&p.x.to_le_bytes());
            h.write(&p.y.to_le_bytes());
        }
        h.write(&self.map.solid);
        h.write(&self.map.ground);
        for v in &self.map.heights {
            h.write(&v.to_le_bytes());
        }
        h.0
    }

    /// Карты в папке: имена подпапок, где есть `map.ron`.
    pub fn list(maps_dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(maps_dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().join("map.ron").exists())
            .filter_map(|e| e.file_name().into_string().ok())
            .collect();
        names.sort();
        names
    }
}

struct Fnv(u64);

impl Default for Fnv {
    fn default() -> Self {
        Fnv(0xcbf2_9ce4_8422_2325)
    }
}

impl Fnv {
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.0 = (self.0 ^ b as u64).wrapping_mul(0x0100_0000_01b3);
        }
    }
}

fn read_png(path: &Path, width: u32, height: u32, depth: u8) -> Result<Vec<u8>, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let decoder = png::Decoder::new(Cursor::new(bytes));
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("картинка слишком большая")?];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    if info.width != width || info.height != height {
        return Err(format!("размер {}×{}, а нужен {width}×{height}", info.width, info.height));
    }
    if info.color_type != png::ColorType::Grayscale || info.bit_depth as u8 != depth {
        return Err(format!("нужна серая картинка {depth} бит"));
    }
    buf.truncate(info.line_size * height as usize);
    Ok(buf)
}

fn write_png(path: &Path, width: u32, height: u32, depth: u8, data: &[u8]) -> Result<(), String> {
    let mut out = Vec::new();
    let mut encoder = png::Encoder::new(&mut out, width, height);
    encoder.set_color(png::ColorType::Grayscale);
    encoder.set_depth(if depth == 16 { png::BitDepth::Sixteen } else { png::BitDepth::Eight });
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(data).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    std::fs::write(path, out).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_and_load_round_trip() {
        let mut file = MapFile::generated("проверка", 96, 3, 5);
        file.map.set_corner_height(10, 12, 3.5);
        file.map.ground[100] = 4;
        let dir = std::env::temp_dir().join(format!("wind_map_test_{}", std::process::id()));
        file.save(&dir).unwrap();
        let back = MapFile::load(&dir).unwrap();
        std::fs::remove_dir_all(&dir).ok();
        assert_eq!(back.name, "проверка");
        assert_eq!(back.map.solid, file.map.solid);
        assert_eq!(back.map.ground, file.map.ground);
        assert_eq!(back.map.heights, file.map.heights);
        assert_eq!(back.layout.capture_points, file.layout.capture_points);
        assert_eq!(back.fingerprint(), file.fingerprint());
        assert!((back.map.height_at(Vec2::new(10.0, 12.0)) - 3.5).abs() < 0.01);
    }

    #[test]
    fn fingerprint_sees_changes() {
        let a = MapFile::generated("a", 64, 2, 1);
        let mut b = a.clone();
        b.map.solid[70] = 9;
        assert_ne!(a.fingerprint(), b.fingerprint());
    }
}
