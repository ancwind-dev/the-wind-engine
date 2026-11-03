//! Трава — по образцу травы хозяина в Blender (`Grass.blend`: меши `Grass_G_Patch` и
//! `Grass_G_Patch_Low`, материал `M_Grass_Ghibli`). Меши выгружены в
//! `data/models/grass/*.glb` вместе с цветами вершин:
//! - `COLOR_0` — ветер (`Wind`): r — гибкость (0 у корня, 1 у кончика), g — фаза травинки,
//!   b — жёсткость (у цветочных стеблей);
//! - `COLOR_1` — оттенок (`GrassShade`): r — яркость, g — сдвиг оттенка, b — цветочки.
//!
//! Лоскут (около 2 × 2 м) ставится на каждую вторую клетку травы у камеры, шахматкой, —
//! со своим поворотом и размером, как копии в файле; лоскуты перекрываются в два слоя,
//! отсюда густой ковёр. Травинка стоит на земле
//! по своему корню (высоты — текстурой на GPU) и растёт, только если под корнем трава.
//! Цвет, свет и ветер — как в материале и узлах ветра файла (`vs_grass` / `fs_grass`).

use crate::gltf;
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec2, Vec4};
use std::path::Path;
use wind_sim::map::Map;

/// Меши лоскутов: высокая трава и низкая.
pub const MESHES: [&str; 2] = ["трава", "трава-низкая"];
/// Какая доля травинок у лоскутов каждой дальности: вблизи — все, дальше — половина и
/// четверть. Травинки в меше — по возрастанию `rand`, а шейдер прячет те, у кого `rand`
/// больше густоты лоскута, — так что дальним хватает начала меша.
pub const LOD_SHARES: [f32; 3] = [1.0, 0.5, 0.25];
/// Покрытия, на которых растёт трава: «трава» и «сухая трава» (см. `GROUND_NAMES`).
const GRASSY: [u8; 2] = [0, 1];

/// Вершина травы.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct GrassVertex {
    /// Вершина в лоскуте, м (Y вверх, середина лоскута — в нуле).
    pub pos: [f32; 3],
    /// Корень травинки (x, z) — по нему высота земли и растёт ли травинка.
    pub root: [f32; 2],
    /// Ветер: гибкость, фаза, жёсткость (`Wind`).
    pub wind: [f32; 3],
    /// Оттенок: яркость, сдвиг оттенка, цветочки (`GrassShade`).
    pub shade: [f32; 3],
    /// Случайное число травинки (см. `LOD_SHARES`).
    pub rand: f32,
}

/// Лоскут на клетке: её середина, поворот (рад), размер, густота (вдаль реже) и
/// признаки: 8 — вокруг сплошной луг (шейдеру не нужно искать край покрытия).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
pub struct GrassPatch {
    pub center: [f32; 2],
    pub angle: f32,
    pub scale: f32,
    pub fade: f32,
    pub flags: f32,
}

/// Меш лоскута, готовый для GPU.
pub struct GrassMesh {
    pub vertices: Vec<GrassVertex>,
    pub indices: Vec<u32>,
    /// Сколько индексов рисовать лоскутам каждой дальности (`LOD_SHARES`).
    pub lods: [u32; 3],
}

/// Меш лоскута из выгрузки Blender: травинки (связные куски) — по возрастанию
/// случайного числа, у каждой — корень (самая низкая вершина).
pub fn build(raw: &gltf::Raw) -> Result<GrassMesh, String> {
    if raw.colors.len() < 2 {
        return Err("нужны цвета вершин Wind (COLOR_0) и GrassShade (COLOR_1)".into());
    }
    let n = raw.positions.len();
    // Травинки — связные куски треугольников.
    let mut parent: Vec<u32> = (0..n as u32).collect();
    fn find(parent: &mut [u32], mut i: u32) -> u32 {
        while parent[i as usize] != i {
            parent[i as usize] = parent[parent[i as usize] as usize];
            i = parent[i as usize];
        }
        i
    }
    for t in raw.indices.as_chunks::<3>().0 {
        let a = find(&mut parent, t[0]);
        for &v in &t[1..] {
            let b = find(&mut parent, v);
            parent[b as usize] = a;
        }
    }
    let comp: Vec<u32> = (0..n as u32).map(|i| find(&mut parent, i)).collect();
    // Корень и случайное число травинки.
    let mut root = vec![(f32::MAX, Vec2::ZERO); n];
    for (i, p) in raw.positions.iter().enumerate() {
        let r = &mut root[comp[i] as usize];
        if p.y < r.0 {
            *r = (p.y, Vec2::new(p.x, p.z));
        }
    }
    let rand = |c: u32| {
        let r = root[c as usize].1;
        let h = (r.x.to_bits() ^ r.y.to_bits().rotate_left(16) ^ c.wrapping_mul(0x9E37_79B9)).wrapping_mul(0x85EB_CA6B);
        ((h ^ (h >> 13)).wrapping_mul(0xC2B2_AE35) >> 8) as f32 / (1u32 << 24) as f32
    };
    // Треугольники — по травинкам в порядке их случайного числа.
    let mut tris: Vec<(f32, [u32; 3])> = raw.indices.as_chunks::<3>().0.iter().map(|t| (rand(comp[t[0] as usize]), *t)).collect();
    tris.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut remap = vec![u32::MAX; n];
    let (mut vertices, mut indices) = (Vec::new(), Vec::new());
    let mut lods = [0u32; 3];
    for (r, t) in &tris {
        for &v in t {
            if remap[v as usize] == u32::MAX {
                remap[v as usize] = vertices.len() as u32;
                let (p, w, s) = (raw.positions[v as usize], raw.colors[0][v as usize], raw.colors[1][v as usize]);
                let base = root[comp[v as usize] as usize].1;
                vertices.push(GrassVertex { pos: p.to_array(), root: base.to_array(), wind: [w.x, w.y, w.z], shade: [s.x, s.y, s.z], rand: *r });
            }
            indices.push(remap[v as usize]);
        }
        for (k, share) in LOD_SHARES.iter().enumerate() {
            if *r < *share {
                lods[k] = indices.len() as u32;
            }
        }
    }
    Ok(GrassMesh { vertices, indices, lods })
}

/// Меши лоскутов из `dir` (`MESHES`); чего нет или что не прочиталось — строкой.
pub fn load(dir: &Path) -> ([Option<GrassMesh>; 2], Vec<String>) {
    let mut notes = Vec::new();
    let meshes = MESHES.map(|name| {
        let path = dir.join(format!("{name}.glb"));
        match std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| gltf::load_raw(&b)).and_then(|raw| build(&raw)) {
            Ok(m) => Some(m),
            Err(e) => {
                notes.push(format!("трава {}: {e}", path.display()));
                None
            }
        }
    });
    (meshes, notes)
}

/// Покрытие клетки, если на ней может расти трава.
fn grassy(map: &Map, x: i32, y: i32) -> Option<u8> {
    let i = map.index(x, y)?;
    (GRASSY.contains(&map.ground[i]) && map.solid[i] == 0).then_some(map.ground[i])
}

/// Сажать ли траву на клетку и как: `None` — рядом травы нет; `Some(true)` — вокруг (на
/// две клетки: край покрытия сдвинут шумом до 0,7 м, а лоскут шире клетки) — сплошь
/// трава; `Some(false)` — край, травинки решит шейдер.
fn kind(map: &Map, x: i32, y: i32) -> Option<bool> {
    let mut uniform = true;
    let mut any = false;
    for dy in -2..=2 {
        for dx in -2..=2 {
            let g = grassy(map, x + dx, y + dy).is_some();
            uniform &= g;
            any |= g && dx.abs() <= 1 && dy.abs() <= 1;
        }
    }
    any.then_some(uniform)
}

fn hash(x: i32, y: i32, salt: u32) -> f32 {
    let h = (x as u32).wrapping_mul(73_856_093) ^ (y as u32).wrapping_mul(19_349_663) ^ salt.wrapping_mul(83_492_791);
    let h = (h ^ (h >> 15)).wrapping_mul(0x2C1B_3C6D);
    ((h ^ (h >> 12)) >> 8) as f32 / (1u32 << 24) as f32
}

/// Лоскуты у камеры: в круге `radius` вокруг `center` и в кадре `view_proj`; к краю круга
/// реже. Подряд — по мешам (`MESHES`) и дальностям (`LOD_SHARES`); возвращает, сколько
/// лоскутов каждого меша каждой дальности. Под водой и в стенах травы нет.
pub fn patches(map: &Map, view_proj: Mat4, center: Vec2, radius: f32, out: &mut Vec<GrassPatch>) -> [[usize; 3]; 2] {
    out.clear();
    let mut lists: [[Vec<GrassPatch>; 3]; 2] = Default::default();
    let (x0, y0) = Map::cell_of(center - Vec2::splat(radius));
    let (x1, y1) = Map::cell_of(center + Vec2::splat(radius));
    let (x0, y0) = (x0.max(0), y0.max(0));
    let (x1, y1) = (x1.min(map.width as i32 - 1), y1.min(map.height as i32 - 1));
    for y in y0..=y1 {
        for x in x0..=x1 {
            // Лоскут шириной 2 м — на каждую вторую клетку (шахматкой): ковёр в два слоя.
            if (x + y) % 2 != 0 {
                continue;
            }
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            let d = p.distance(center);
            if d > radius {
                continue;
            }
            let Some(uniform) = kind(map, x, y) else { continue };
            let h = map.height_at(p);
            if map.water.is_some_and(|w| h < w - 0.2) {
                continue;
            }
            let clip = view_proj * Vec4::new(p.x, h + 0.4, p.y, 1.0);
            if clip.w <= 0.0 || (clip.x / clip.w).abs() > 1.1 || (clip.y / clip.w).abs() > 1.15 {
                continue;
            }
            let fade = 1.0 - smoothstep(radius * 0.2, radius, d);
            // Высокая с низкой вперемешку, как в файле, — везде одинаково: если делить по
            // покрытию, низкая трава на «сухой» ложится его клетками — пятнами со ступенями.
            let mesh = if hash(x, y, 1) < 0.7 { 0 } else { 1 };
            let lod = LOD_SHARES.iter().rposition(|&s| s >= fade).unwrap_or(0);
            lists[mesh][lod].push(GrassPatch {
                center: [p.x, p.y],
                angle: hash(x, y, 2) * std::f32::consts::TAU,
                scale: 0.88 + hash(x, y, 3) * 0.3,
                fade,
                flags: if uniform { 8.0 } else { 0.0 },
            });
        }
    }
    let counts = lists.each_ref().map(|m| m.each_ref().map(Vec::len));
    for list in lists.into_iter().flatten() {
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

    /// Две травинки: полоска из двух треугольников и одиночный треугольник.
    fn raw() -> gltf::Raw {
        let positions = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.1, 0.0, 0.0),
            Vec3::new(0.0, 0.5, 0.0),
            Vec3::new(0.1, 0.5, 0.0),
            Vec3::new(0.5, 0.02, 0.5),
            Vec3::new(0.6, 0.0, 0.5),
            Vec3::new(0.55, 0.3, 0.5),
        ];
        let wind = positions.iter().map(|p| Vec4::new(p.y * 2.0, 0.3, 0.0, 1.0)).collect();
        let shade = positions.iter().map(|_| Vec4::new(0.5, 0.5, 0.0, 1.0)).collect();
        gltf::Raw { positions, colors: vec![wind, shade], indices: vec![0, 1, 2, 1, 3, 2, 4, 5, 6] }
    }

    #[test]
    fn blades_get_their_root_and_meshes_order_for_lods() {
        let m = build(&raw()).unwrap();
        assert_eq!(m.vertices.len(), 7);
        assert_eq!(m.indices.len(), 9);
        // У каждой травинки корень — её нижняя вершина.
        for v in &m.vertices {
            let expect = if v.pos[0] < 0.3 { [0.0, 0.0] } else { [0.6, 0.5] };
            assert_eq!(v.root, expect, "{v:?}");
        }
        // Травинки — по возрастанию случайного числа; вблизи рисуются все.
        assert!(m.vertices.windows(2).all(|w| w[0].rand <= w[1].rand));
        assert_eq!(m.lods[0], 9);
        assert!(m.lods[1] <= m.lods[0] && m.lods[2] <= m.lods[1]);
        // Без цветов вершин — понятная ошибка.
        assert!(build(&gltf::Raw { colors: vec![], ..raw() }).is_err());
    }

    #[test]
    fn real_grass_from_blender_loads() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/models/grass");
        if !dir.join("трава.glb").exists() {
            return;
        }
        let (meshes, notes) = load(&dir);
        assert!(notes.is_empty(), "{notes:?}");
        let tall = meshes[0].as_ref().unwrap();
        println!("трава: {} вершин, {} треугольников, дальности {:?}", tall.vertices.len(), tall.indices.len() / 3, tall.lods);
        assert!(tall.vertices.iter().all(|v| v.pos[1] >= -0.01 && v.pos[1] < 1.2));
        // Ветер у корня — ноль, у кончиков — до единицы.
        assert!(tall.vertices.iter().filter(|v| v.pos[1] < 0.01).all(|v| v.wind[0] < 0.2));
        assert!(tall.vertices.iter().any(|v| v.wind[0] > 0.9));
        // Цветочки есть.
        assert!(tall.vertices.iter().any(|v| v.shade[2] > 0.2));
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
        let counts = patches(&map, proj * view, center, 20.0, &mut out);
        assert!(!out.is_empty());
        assert_eq!(counts.iter().flatten().sum::<usize>(), out.len());
        // Дальним хватает начала меша: их густота не больше доли травинок.
        let mut at = 0;
        for mesh in counts {
            for (lod, &n) in mesh.iter().enumerate() {
                for p in &out[at..at + n] {
                    assert!(p.fade <= LOD_SHARES[lod] + 1e-3);
                }
                at += n;
            }
        }
        for p in &out {
            let c = Vec2::from(p.center);
            assert!(c.distance(center) <= 20.0);
            assert!((0.88..=1.18).contains(&p.scale));
            // Середина дороги — без травы.
            assert!(!(31.0..=33.0).contains(&p.center[1]), "трава посреди дороги: {:?}", p.center);
        }
        // Посреди луга шейдеру край не нужен, у дороги — нужен.
        let flags = |x: f32, y: f32| out.iter().find(|p| p.center == [x, y]).map(|p| p.flags);
        assert_eq!(flags(32.5, 24.5), Some(8.0));
        assert_eq!(flags(32.5, 28.5), Some(0.0));
        // Шахматкой: на соседней клетке лоскута нет.
        assert_eq!(flags(33.5, 24.5), None);
        // Вдаль — реже.
        let far = out.iter().filter(|p| Vec2::from(p.center).distance(center) > 18.5).map(|p| p.fade).fold(1.0, f32::min);
        assert!(far < 0.2);
    }
}
