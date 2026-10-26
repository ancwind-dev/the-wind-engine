//! Меш карты по чанкам 32×32. Целая стена — столбик на клетку. Повреждённая
//! дробится на столбики по 1/3 м разной высоты — край рваный, а чем сильнее
//! повреждение, тем ниже и темнее. Где стена рухнула — низкая груда (руины).
//! Повреждение считаем от прочности, какой клетка была, когда мы её увидели впервые:
//! у целого забора прочность меньше, чем у целого дома.
//!
//! Перестраиваются только чанки, версия которых изменилась (и их соседи —
//! у них могли открыться боковые грани).

use crate::voxel::{Mesh, Vertex};
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

/// На сколько столбиков по стороне дробится повреждённая клетка и клетка с руинами.
const SPLIT: i32 = 3;

/// Что мешер помнит о клетках: прочность целой стены и где остались руины.
pub struct Walls<'a> {
    /// Прочность, какой клетка была впервые (или самая большая с тех пор).
    pub full: &'a [u8],
    /// Здесь стена рухнула: прочность той стены (для цвета), 0 — руин нет.
    pub rubble: &'a [u8],
}

impl Walls<'_> {
    /// Насколько повреждена стена: 0 — целая, 1 — почти разрушена.
    fn damage(&self, map: &Map, i: usize) -> f32 {
        let (hp, full) = (map.solid[i], self.full[i].max(map.solid[i]));
        if hp == INDESTRUCTIBLE || full == 0 { 0.0 } else { 1.0 - hp as f32 / full as f32 }
    }
}

/// Низ клетки и высота земли в её середине, м. Низ — чуть ниже самого низкого угла,
/// чтобы на склоне между стеной и землёй не было щели.
fn ground(map: &Map, x: i32, y: i32) -> (f32, f32) {
    let corners = [map.corner_height(x, y), map.corner_height(x + 1, y), map.corner_height(x, y + 1), map.corner_height(x + 1, y + 1)];
    let low = corners.iter().copied().fold(f32::INFINITY, f32::min);
    (low - 0.05, corners.iter().sum::<f32>() / 4.0)
}

/// Низ и верх целого столбика стены в клетке, м. У повреждённой — самый низкий
/// из её мелких столбиков: соседи рисуют свои боковые грани до него.
fn column(map: &Map, walls: &Walls, x: i32, y: i32) -> Option<(f32, f32)> {
    let i = map.index(x, y)?;
    let hp = map.solid[i];
    if hp == EMPTY {
        return None;
    }
    let (base, mid) = ground(map, x, y);
    let full = wall_height(walls.full[i].max(hp));
    let damage = walls.damage(map, i);
    let top = if damage > 0.0 { (0..SPLIT * SPLIT).map(|k| broken_top(full, damage, x, y, k)).fold(f32::INFINITY, f32::min) } else { full };
    Some((base, mid + top))
}

/// Высота мелкого столбика `k` повреждённой клетки над землёй.
fn broken_top(full: f32, damage: f32, x: i32, y: i32, k: i32) -> f32 {
    let u = (hash(x * SPLIT + k % SPLIT, y * SPLIT + k / SPLIT) % 1000) as f32 / 1000.0;
    // Чем сильнее повреждение, тем глубже и чаще выбоины.
    let bite = damage * (0.35 + 1.3 * u);
    full * (1.0 - bite).clamp(0.08, 1.0)
}

/// Коробка из верха и четырёх боковых граней (низ не виден). Низ боков темнее —
/// дешёвая имитация затенения у земли.
fn add_box(mesh: &mut Mesh, (x0, z0, x1, z1): (f32, f32, f32, f32), base: f32, top: f32, c: [f32; 3]) {
    let v = Vec3::new;
    mesh.quad([v(x0, top, z0), v(x0, top, z1), v(x1, top, z1), v(x1, top, z0)], [0, 127, 0], [rgba(c, 1.0); 4]);
    let (low, up) = (rgba(c, 0.6), rgba(c, 0.92));
    let sides = [
        ([v(x1, base, z0), v(x1, top, z0), v(x1, top, z1), v(x1, base, z1)], [127, 0, 0]),
        ([v(x0, base, z1), v(x0, top, z1), v(x0, top, z0), v(x0, base, z0)], [-127, 0, 0]),
        ([v(x1, base, z1), v(x1, top, z1), v(x0, top, z1), v(x0, base, z1)], [0, 0, 127]),
        ([v(x0, base, z0), v(x0, top, z0), v(x1, top, z0), v(x1, base, z0)], [0, 0, -127]),
    ];
    for (corners, normal) in sides {
        mesh.quad(corners, normal, [low, up, up, low]);
    }
}

/// Повреждённая клетка: столбики 1/3 м разной высоты.
fn add_broken(mesh: &mut Mesh, map: &Map, walls: &Walls, x: i32, y: i32, i: usize) {
    let (base, mid) = ground(map, x, y);
    let damage = walls.damage(map, i);
    let full = wall_height(walls.full[i].max(map.solid[i]));
    let c = wall_color(walls.full[i].max(map.solid[i]), x, y).map(|v| v * (1.0 - 0.35 * damage));
    let step = 1.0 / SPLIT as f32;
    for k in 0..SPLIT * SPLIT {
        let (sx, sz) = (x as f32 + (k % SPLIT) as f32 * step, y as f32 + (k / SPLIT) as f32 * step);
        let shade = 0.9 + 0.1 * (hash(x * 7 + k, y * 5 - k) % 100) as f32 / 100.0;
        add_box(mesh, (sx, sz, sx + step, sz + step), base, mid + broken_top(full, damage, x, y, k), c.map(|v| v * shade));
    }
}

/// Руины на месте рухнувшей стены: низкие кирпичи, ходить можно.
fn add_rubble(mesh: &mut Mesh, map: &Map, x: i32, y: i32, was: u8) {
    let (base, mid) = ground(map, x, y);
    let c = wall_color(was, x, y).map(|v| v * 0.62);
    let step = 1.0 / SPLIT as f32;
    for k in 0..SPLIT * SPLIT {
        let u = (hash(x * SPLIT + k % SPLIT + 911, y * SPLIT + k / SPLIT) % 1000) as f32 / 1000.0;
        if u < 0.25 {
            continue;
        }
        let (sx, sz) = (x as f32 + (k % SPLIT) as f32 * step, y as f32 + (k / SPLIT) as f32 * step);
        add_box(mesh, (sx, sz, sx + step, sz + step), base, mid + 0.06 + 0.3 * (u - 0.25), c.map(|v| v * (0.85 + 0.3 * u)));
    }
}

/// Стены чанка: столбики на рельефе, рваные повреждённые клетки, руины.
pub fn mesh_chunk(map: &Map, walls: &Walls, chunk: usize) -> Mesh {
    let mut mesh = Mesh::default();
    let (x0, y0, w, h) = map.chunk_rect(chunk);
    for y in y0 as i32..(y0 + h) as i32 {
        for x in x0 as i32..(x0 + w) as i32 {
            let i = map.index(x, y).unwrap();
            let hp = map.solid[i];
            if hp == EMPTY {
                if walls.rubble[i] != 0 {
                    add_rubble(&mut mesh, map, x, y, walls.rubble[i]);
                }
                continue;
            }
            if walls.damage(map, i) > 0.0 {
                add_broken(&mut mesh, map, walls, x, y, i);
                continue;
            }
            let Some((base, top)) = column(map, walls, x, y) else { continue };
            let c = wall_color(walls.full[i].max(hp), x, y);
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
                let neighbor = column(map, walls, x + dx, y + dy);
                let low = neighbor.map_or(base, |(_, t)| t.max(base));
                if low >= top {
                    continue;
                }
                let bottom = rgba(c, if neighbor.is_none() { 0.55 } else { 0.8 });
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

/// Земля чанка: сетка по углам клеток, нормали — по соседним высотам.
pub fn terrain_chunk(map: &Map, chunk: usize) -> Mesh {
    let mut mesh = Mesh::default();
    let (x0, y0, w, h) = map.chunk_rect(chunk);
    let (x0, y0, w, h) = (x0 as i32, y0 as i32, w as i32, h as i32);
    for y in y0..=y0 + h {
        for x in x0..=x0 + w {
            let n = Vec3::new(
                map.corner_height(x - 1, y) - map.corner_height(x + 1, y),
                2.0,
                map.corner_height(x, y - 1) - map.corner_height(x, y + 1),
            )
            .normalize();
            mesh.vertices.push(Vertex {
                pos: [x as f32, map.corner_height(x, y), y as f32],
                normal: [(n.x * 127.0) as i8, (n.y * 127.0) as i8, (n.z * 127.0) as i8, 0],
                color: [255, 255, 255, 255],
            });
        }
    }
    let row = (w + 1) as u32;
    for y in 0..h as u32 {
        for x in 0..w as u32 {
            let a = y * row + x;
            let (b, c, d) = (a + row, a + row + 1, a + 1);
            // Против часовой, если смотреть сверху.
            mesh.indices.extend_from_slice(&[a, b, c, a, c, d]);
        }
    }
    mesh
}

/// Меши одного чанка карты.
pub struct ChunkMeshes {
    /// `None` — стен в чанке нет.
    pub walls: Option<Mesh>,
    pub terrain: Mesh,
}

/// Помнит, какие версии чанков уже превращены в меши.
#[derive(Default)]
pub struct MapMesher {
    key: u64,
    seen: Vec<u32>,
    /// Прочность клеток в прошлый раз — по ней видно, что сломалось.
    prev: Vec<u8>,
    /// Прочность целой стены и руины (см. [`Walls`]).
    full: Vec<u8>,
    rubble: Vec<u8>,
}

/// Что поменялось в мешах карты с прошлого кадра.
pub struct MapUpdate {
    /// Карта другая целиком — старые меши выбросить.
    pub reset: bool,
    pub chunks: Vec<(u32, ChunkMeshes)>,
    /// Клетки, которые стали слабее: из них летят обломки.
    pub broken: Vec<BrokenCell>,
}

/// Клетка стены, которой досталось: где, насколько и какого цвета была стена.
#[derive(Clone, Copy, Debug)]
pub struct BrokenCell {
    pub x: i32,
    pub y: i32,
    /// Сколько прочности потеряла, долей целой стены.
    pub lost: f32,
    /// Рухнула совсем.
    pub destroyed: bool,
    /// Цвет стены (sRGB 0..1).
    pub color: [f32; 3],
}

/// Если за раз сломалось больше клеток, это не взрыв, а догрузка карты
/// (подключились посреди матча) — обломки не сыплем.
const MAX_BROKEN_AT_ONCE: usize = 400;

impl MapMesher {
    /// `key` различает карты (например, локальную и сетевую).
    pub fn update(&mut self, map: &Map, key: u64) -> MapUpdate {
        let n = map.chunk_revisions.len();
        let reset = key != self.key || self.seen.len() != n;
        if reset {
            self.key = key;
            self.seen = vec![u32::MAX; n];
            self.prev = map.solid.clone();
            self.full = map.solid.clone();
            self.rubble = vec![0; map.solid.len()];
        }
        let mut broken = Vec::new();
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
        // Сначала — что поменялось в клетках: руины, прочность целых стен, обломки.
        for c in (0..n).filter(|&c| dirty[c] && self.seen[c] != map.chunk_revisions[c]) {
            let (x0, y0, w, h) = map.chunk_rect(c);
            for y in y0..y0 + h {
                for x in x0..x0 + w {
                    let i = (y * map.width + x) as usize;
                    let (before, now) = (self.prev[i], map.solid[i]);
                    if before == now {
                        continue;
                    }
                    self.prev[i] = now;
                    if now > before {
                        // Стену поставили (редактор) — она целая.
                        self.full[i] = now;
                        self.rubble[i] = 0;
                        continue;
                    }
                    let full = self.full[i].max(before);
                    if now == EMPTY {
                        self.rubble[i] = full;
                    }
                    broken.push(BrokenCell {
                        x: x as i32,
                        y: y as i32,
                        lost: (before - now) as f32 / full as f32,
                        destroyed: now == EMPTY,
                        color: wall_color(full, x as i32, y as i32).map(|v| v / 255.0),
                    });
                }
            }
        }
        if broken.len() > MAX_BROKEN_AT_ONCE {
            broken.clear();
        }
        let walls = Walls { full: &self.full, rubble: &self.rubble };
        let chunks = (0..n)
            .filter(|&c| dirty[c])
            .map(|c| {
                self.seen[c] = map.chunk_revisions[c];
                let mesh = mesh_chunk(map, &walls, c);
                let terrain = terrain_chunk(map, c);
                (c as u32, ChunkMeshes { walls: (!mesh.indices.is_empty()).then_some(mesh), terrain })
            })
            .collect();
        MapUpdate { reset, chunks, broken }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lonely_wall_has_five_faces() {
        let mut map = Map::new(32, 32);
        map.set(5, 5, 100);
        let (full, rubble) = (map.solid.clone(), vec![0; map.solid.len()]);
        assert_eq!(mesh_chunk(&map, &Walls { full: &full, rubble: &rubble }, 0).quads(), 5);
    }

    #[test]
    fn damage_breaks_walls_and_leaves_rubble() {
        let mut map = Map::new(64, 64);
        for x in 10..20 {
            map.set(x, 10, 120);
        }
        let mut mesher = MapMesher::default();
        let whole = mesher.update(&map, 1);
        assert!(whole.broken.is_empty());
        let quads = |u: &MapUpdate| u.chunks.iter().filter_map(|(_, m)| m.walls.as_ref()).map(|m| m.quads()).sum::<usize>();
        let before = quads(&whole);
        // Взрыв посередине: в центре стена рухнула, по краям — повреждена.
        map.damage_circle(wind_sim::Vec2::new(15.0, 10.5), 3.0, 200.0);
        let after = mesher.update(&map, 1);
        assert!(after.broken.iter().any(|b| b.destroyed), "в центре стена должна рухнуть");
        assert!(after.broken.iter().any(|b| !b.destroyed && b.lost > 0.0), "по краям стена должна быть повреждена");
        assert!(mesher.rubble.iter().any(|&r| r != 0), "на месте стены должны остаться руины");
        // Рваные клетки и руины — из мелких столбиков, граней больше.
        assert!(quads(&after) > before, "{} > {before}", quads(&after));
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
