//! Меш карты по чанкам 32×32. Каждая клетка со стеной — столбик, высота
//! которого зависит от прочности: повреждённая стена ниже целой.
//!
//! Перестраиваются только чанки, версия которых изменилась (и их соседи —
//! у них могли открыться боковые грани).

use crate::voxel::Mesh;
use glam::Vec3;
use wind_sim::map::{EMPTY, INDESTRUCTIBLE, Map};

/// Прочность целой стены дома; выше — тоже «целая».
const FULL: f32 = 120.0;

pub fn wall_height(hp: u8) -> f32 {
    match hp {
        EMPTY => 0.0,
        INDESTRUCTIBLE => 2.6,
        hp => 0.35 + 2.65 * (hp as f32 / FULL).min(1.0),
    }
}

fn hash(x: i32, y: i32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841);
    h ^= h >> 13;
    h.wrapping_mul(0x85eb_ca6b)
}

fn wall_color(hp: u8, x: i32, y: i32) -> [f32; 3] {
    // Небольшой разброс по клеткам — так стены выглядят воксельными, а не пластиковыми.
    let noise = (hash(x, y) % 17) as f32 - 8.0;
    let base = if hp == INDESTRUCTIBLE {
        [100.0, 100.0, 96.0]
    } else {
        let t = (hp as f32 / FULL).min(1.0);
        let lerp = |a: f32, b: f32| a + (b - a) * t;
        [lerp(112.0, 168.0), lerp(92.0, 158.0), lerp(70.0, 136.0)]
    };
    base.map(|c| (c + noise).clamp(0.0, 255.0))
}

fn rgba(c: [f32; 3], shade: f32) -> [u8; 4] {
    [(c[0] * shade) as u8, (c[1] * shade) as u8, (c[2] * shade) as u8, 255]
}

pub fn mesh_chunk(map: &Map, chunk: usize) -> Mesh {
    let mut mesh = Mesh::default();
    let (x0, y0, w, h) = map.chunk_rect(chunk);
    let height_at = |x: i32, y: i32| map.index(x, y).map_or(0.0, |i| wall_height(map.solid[i]));
    for y in y0 as i32..(y0 + h) as i32 {
        for x in x0 as i32..(x0 + w) as i32 {
            let hp = map.solid[map.index(x, y).unwrap()];
            let top = wall_height(hp);
            if top == 0.0 {
                continue;
            }
            let c = wall_color(hp, x, y);
            let (fx, fz) = (x as f32, y as f32);
            let v = |px: f32, py: f32, pz: f32| Vec3::new(px, py, pz);
            mesh.quad(
                [v(fx, top, fz), v(fx, top, fz + 1.0), v(fx + 1.0, top, fz + 1.0), v(fx + 1.0, top, fz)],
                [0, 127, 0],
                [rgba(c, 1.0); 4],
            );
            // Боковые грани — только там, где сосед ниже. Низ темнее: дешёвая имитация затенения у земли.
            let sides: [((i32, i32), [i8; 3]); 4] =
                [((1, 0), [127, 0, 0]), ((-1, 0), [-127, 0, 0]), ((0, 1), [0, 0, 127]), ((0, -1), [0, 0, -127])];
            for ((dx, dy), normal) in sides {
                let low = height_at(x + dx, y + dy);
                if low >= top {
                    continue;
                }
                let bottom = rgba(c, if low == 0.0 { 0.55 } else { 0.8 });
                let upper = rgba(c, 0.95);
                let corners = match (dx, dy) {
                    (1, 0) => [v(fx + 1.0, low, fz), v(fx + 1.0, top, fz), v(fx + 1.0, top, fz + 1.0), v(fx + 1.0, low, fz + 1.0)],
                    (-1, 0) => [v(fx, low, fz + 1.0), v(fx, top, fz + 1.0), v(fx, top, fz), v(fx, low, fz)],
                    (0, 1) => [v(fx + 1.0, low, fz + 1.0), v(fx + 1.0, top, fz + 1.0), v(fx, top, fz + 1.0), v(fx, low, fz + 1.0)],
                    _ => [v(fx, low, fz), v(fx, top, fz), v(fx + 1.0, top, fz), v(fx + 1.0, low, fz)],
                };
                mesh.quad(corners, normal, [bottom, upper, upper, bottom]);
            }
        }
    }
    mesh
}

/// Помнит, какие версии чанков уже превращены в меши.
#[derive(Default)]
pub struct MapMesher {
    key: u64,
    seen: Vec<u32>,
}

/// Что поменялось в мешах карты с прошлого кадра.
pub struct MapUpdate {
    /// Карта другая целиком — старые меши выбросить.
    pub reset: bool,
    /// (чанк, меш); `None` — чанк теперь пуст.
    pub chunks: Vec<(u32, Option<Mesh>)>,
}

impl MapMesher {
    /// `key` различает карты (например, локальную и сетевую).
    pub fn update(&mut self, map: &Map, key: u64) -> MapUpdate {
        let n = map.chunk_revisions.len();
        let reset = key != self.key || self.seen.len() != n;
        if reset {
            self.key = key;
            self.seen = vec![u32::MAX; n];
        }
        let cx = map.chunks_x() as usize;
        let mut dirty = vec![false; n];
        for c in 0..n {
            if self.seen[c] != map.chunk_revisions[c] {
                dirty[c] = true;
                if !reset {
                    // Соседям тоже: у них могли открыться боковые грани.
                    let (x, y) = (c % cx, c / cx);
                    for (nx, ny) in [(x.wrapping_sub(1), y), (x + 1, y), (x, y.wrapping_sub(1)), (x, y + 1)] {
                        if nx < cx && ny * cx + nx < n {
                            dirty[ny * cx + nx] = true;
                        }
                    }
                }
            }
        }
        let chunks = (0..n)
            .filter(|&c| dirty[c])
            .map(|c| {
                self.seen[c] = map.chunk_revisions[c];
                let mesh = mesh_chunk(map, c);
                (c as u32, (!mesh.indices.is_empty()).then_some(mesh))
            })
            .collect();
        MapUpdate { reset, chunks }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lonely_wall_has_five_faces() {
        let mut map = Map::new(32, 32);
        map.set(5, 5, 100);
        assert_eq!(mesh_chunk(&map, 0).quads(), 5);
    }

    #[test]
    fn only_damaged_chunks_are_rebuilt() {
        let mut map = Map::new(128, 128);
        map.set(40, 40, 100);
        let mut mesher = MapMesher::default();
        let first = mesher.update(&map, 1);
        assert!(first.reset);
        assert_eq!(first.chunks.len(), 16);
        assert!(mesher.update(&map, 1).chunks.is_empty());
        map.damage_circle(wind_sim::Vec2::new(40.5, 40.5), 2.0, 50.0);
        let after = mesher.update(&map, 1);
        // Сам чанк и четыре соседа.
        assert_eq!(after.chunks.len(), 5);
    }
}
