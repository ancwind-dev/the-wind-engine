//! Трава в духе Ghibli: сплошной ковёр тонких лент, а не отдельные кусты.
//!
//! Приёмы — из разбора «аниме-травы» в Blender:
//! - свет по нормали земли, а не травинки: поле светится как одна мягкая поверхность;
//! - цвет — крупные пятна шума по миру (тёмная зелень → зелень → жёлто-зелёный), у каждой
//!   травинки чуть своя яркость;
//! - трава не отбрасывает тень;
//! - ветер — бегущие по полю волны шума, травинка гнётся от корня.
//!
//! Моделей нет: движок сам строит лоскут (клетка 1×1 м, `BLADES` лент) и ставит его
//! экземплярами на клетки у камеры, где рядом есть трава. Расти ли каждой травинке,
//! решает шейдер по той же «рисованной» границе покрытия, что и у земли (`vs_grass`).

use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec2, Vec4};
use wind_sim::map::Map;
use wind_sim::rng::Rng;

/// Травинок в лоскуте 1×1 м.
pub const BLADES: usize = 72;
/// Покрытия, на которых растёт трава: «трава» и «сухая трава» (см. `GROUND_NAMES`).
const GRASSY: [u8; 2] = [0, 1];

/// Вершина травинки: корень в клетке, смещение от корня (без ветра), доля высоты,
/// случайное число травинки и её высота.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GrassVertex {
    pub root: [f32; 2],
    pub offset: [f32; 3],
    pub t: f32,
    pub rand: f32,
    pub height: f32,
}

/// Лоскут на клетке: её угол, высоты четырёх углов, насколько густо (вдаль — реже)
/// и признаки: биты 0–2 — поворот (четыре поворота и зеркало — чтобы лоскуты не
/// повторялись), бит 3 — вокруг сплошной луг (шейдеру не нужно искать край), бит 4 —
/// и он сухой.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct GrassPatch {
    pub origin: [f32; 2],
    /// Углы: (x, y), (x + 1, y), (x, y + 1), (x + 1, y + 1).
    pub heights: [f32; 4],
    pub fade: f32,
    pub variant: f32,
}

/// Индексов на травинку.
pub const BLADE_INDICES: u32 = 15;

/// Сколько травинок рисовать у лоскутов каждой дальности: вблизи — все, дальше — две
/// трети и треть. Травинки в меше идут по возрастанию `rand`, а шейдер прячет те, у
/// кого `rand` больше густоты лоскута, — так что дальним хватает начала меша.
pub const LOD_BLADES: [usize; 3] = [BLADES, BLADES * 2 / 3, BLADES / 3];

/// Меш лоскута: `BLADES` лент по 3 отрезка и острию — 7 вершин и 5 треугольников.
/// Травинки — по возрастанию `rand` (см. `LOD_BLADES`).
pub fn patch_mesh(seed: u64) -> (Vec<GrassVertex>, Vec<u32>) {
    let mut rng = Rng::new(seed);
    let (mut vertices, mut indices) = (Vec::new(), Vec::new());
    for blade in 0..BLADES {
        let root = [rng.range(0.0, 1.0), rng.range(0.0, 1.0)];
        let yaw = rng.range(0.0, std::f32::consts::TAU);
        let dir = Vec2::from_angle(yaw);
        let side = dir.perp();
        let height = rng.range(0.36, 0.66);
        let width = rng.range(0.08, 0.13);
        // Наклон кончика — доля высоты; лента гнётся дугой.
        let lean = rng.range(0.1, 0.4);
        // Равномерно по порядку: травинка k — около k / BLADES.
        let rand = (blade as f32 + rng.range(0.0, 1.0)) / BLADES as f32;
        let base = vertices.len() as u32;
        for (k, t) in [0.0f32, 0.4, 0.75, 1.0].into_iter().enumerate() {
            let along = dir * lean * height * t.powf(1.7);
            let y = height * t * (1.0 - 0.15 * lean * t);
            let w = width * (1.0 - t).powf(0.8);
            let sides: &[f32] = if k == 3 { &[0.0] } else { &[-0.5, 0.5] };
            for &s in sides {
                let off = along + side * w * s;
                vertices.push(GrassVertex { root, offset: [off.x, y, off.y], t, rand, height });
            }
        }
        for [a, b, c] in [[0, 1, 3], [0, 3, 2], [2, 3, 5], [2, 5, 4], [4, 5, 6]] {
            indices.extend([base + a, base + b, base + c]);
        }
    }
    (vertices, indices)
}

/// Покрытие клетки, если на ней может расти трава.
fn grassy(map: &Map, x: i32, y: i32) -> Option<u8> {
    let i = map.index(x, y)?;
    (GRASSY.contains(&map.ground[i]) && map.solid[i] == 0).then_some(map.ground[i])
}

/// Сажать ли траву на клетку и как: `None` — рядом травы нет; `Some(Some(m))` — вокруг
/// (с соседями по диагонали: край покрытия сдвинут шумом до 0,7 м) сплошь покрытие `m`;
/// `Some(None)` — край, травинки решит шейдер.
fn kind(map: &Map, x: i32, y: i32) -> Option<Option<u8>> {
    let center = grassy(map, x, y);
    let mut uniform = center;
    let mut any = center.is_some();
    for dy in -1..=1 {
        for dx in -1..=1 {
            let g = grassy(map, x + dx, y + dy);
            if g != uniform {
                uniform = None;
            }
            any |= g.is_some() && (dx == 0 || dy == 0);
        }
    }
    any.then_some(uniform)
}

/// Лоскуты у камеры: в круге `radius` вокруг `center` и в кадре `view_proj`; к краю
/// круга реже. Под водой и в стенах травы нет. По дальности (`LOD_BLADES`) — подряд:
/// возвращает, сколько лоскутов каждой дальности.
pub fn patches(map: &Map, view_proj: Mat4, center: Vec2, radius: f32, out: &mut Vec<GrassPatch>) -> [usize; 3] {
    out.clear();
    let mut lods: [Vec<GrassPatch>; 3] = Default::default();
    let (x0, y0) = Map::cell_of(center - Vec2::splat(radius));
    let (x1, y1) = Map::cell_of(center + Vec2::splat(radius));
    let (x0, y0) = (x0.max(0), y0.max(0));
    let (x1, y1) = (x1.min(map.width as i32 - 1), y1.min(map.height as i32 - 1));
    for y in y0..=y1 {
        for x in x0..=x1 {
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            let d = p.distance(center);
            if d > radius {
                continue;
            }
            let Some(uniform) = kind(map, x, y) else { continue };
            let heights = [map.corner_height(x, y), map.corner_height(x + 1, y), map.corner_height(x, y + 1), map.corner_height(x + 1, y + 1)];
            if map.water.is_some_and(|w| heights.iter().all(|&h| h < w)) {
                continue;
            }
            let mid = heights.iter().sum::<f32>() * 0.25 + 0.3;
            let clip = view_proj * Vec4::new(p.x, mid, p.y, 1.0);
            if clip.w <= 0.0 || (clip.x / clip.w).abs() > 1.08 || (clip.y / clip.w).abs() > 1.12 {
                continue;
            }
            let fade = 1.0 - smoothstep(radius * 0.3, radius, d);
            let turn = ((x as u32).wrapping_mul(73_856_093) ^ (y as u32).wrapping_mul(19_349_663)) >> 7 & 7;
            let flags = match uniform {
                Some(m) => 8 + 16 * (m == 1) as u32,
                None => 0,
            };
            let lod = LOD_BLADES.iter().rposition(|&n| n as f32 >= fade * BLADES as f32).unwrap_or(0);
            lods[lod].push(GrassPatch { origin: [x as f32, y as f32], heights, fade, variant: (turn + flags) as f32 });
        }
    }
    let counts = lods.each_ref().map(Vec::len);
    for list in lods {
        out.extend(list);
    }
    counts
}

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;
    use glam::camera::rh::{proj::directx as proj, view};

    #[test]
    fn patch_mesh_has_blades_rooted_in_the_cell() {
        let (v, i) = patch_mesh(1);
        assert_eq!(v.len(), BLADES * 7);
        assert_eq!(i.len(), BLADES * 15);
        assert!(i.iter().all(|&k| (k as usize) < v.len()));
        for vx in &v {
            assert!((0.0..1.0).contains(&vx.root[0]) && (0.0..1.0).contains(&vx.root[1]));
            if vx.t == 0.0 {
                assert_eq!(vx.offset[1], 0.0, "корень — на земле");
            }
            assert!(vx.offset[1] <= vx.height + 1e-4);
        }
    }

    #[test]
    fn patches_grow_on_grass_near_the_camera_and_not_on_roads_or_far_away() {
        let mut map = Map::new(64, 64);
        // Дорога (земля) поперёк карты.
        for x in 0..64 {
            for y in 30..34 {
                let i = map.index(x, y).unwrap();
                map.ground[i] = 2;
            }
        }
        let center = Vec2::new(32.0, 32.0);
        let view = view::look_at_mat4(Vec3::new(32.0, 60.0, 32.1), Vec3::new(32.0, 0.0, 32.0), Vec3::Y);
        let proj = proj::perspective(1.2, 1.0, 1.0, 200.0);
        let mut out = Vec::new();
        let lods = patches(&map, proj * view, center, 20.0, &mut out);
        assert!(!out.is_empty());
        assert_eq!(lods.iter().sum::<usize>(), out.len());
        // Дальним хватает начала меша: их густота не больше доли травинок.
        let mut at = 0;
        for (lod, &n) in lods.iter().enumerate() {
            for p in &out[at..at + n] {
                assert!(p.fade * BLADES as f32 <= LOD_BLADES[lod] as f32 + 1e-3);
            }
            at += n;
        }
        // Посреди луга шейдеру край не нужен, у дороги — нужен.
        let flags = |x: f32, y: f32| out.iter().find(|p| p.origin == [x, y]).map(|p| p.variant as u32 & !7);
        assert_eq!(flags(32.0, 24.0), Some(8));
        assert_eq!(flags(32.0, 29.0), Some(0));
        for p in &out {
            let c = Vec2::new(p.origin[0] + 0.5, p.origin[1] + 0.5);
            assert!(c.distance(center) <= 20.0);
            // Середина дороги — без травы (край — на усмотрение шейдера).
            assert!(!(31.0..=32.0).contains(&p.origin[1]), "трава посреди дороги: {:?}", p.origin);
        }
        // Вдаль — реже.
        let near = out.iter().find(|p| p.origin == [32.0, 26.0]).unwrap();
        let far = out.iter().filter(|p| Vec2::from(p.origin).distance(center) > 18.5).map(|p| p.fade).fold(1.0, f32::min);
        assert!(near.fade > 0.99 && far < 0.2);
    }
}

