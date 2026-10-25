//! Временная процедурная карта для вехи 0: базы по краям, точки захвата
//! посередине, между ними дома, заборы и скалы. Позже карты будут рисоваться
//! в редакторе.

use crate::map::{INDESTRUCTIBLE, Map};
use crate::rng::Rng;
use glam::Vec2;

pub const HOUSE_WALL: u8 = 120;
pub const FENCE: u8 = 40;

pub struct Layout {
    /// База каждой команды: там появляются солдаты.
    pub bases: [Vec2; 2],
    pub capture_points: Vec<Vec2>,
}

pub fn generate(size: u32, capture_points: u32, seed: u64) -> (Map, Layout) {
    let mut rng = Rng::new(seed);
    let mut map = Map::new(size, size);
    let s = size as f32;
    let n = size as i32;

    let margin = (s * 0.05).max(16.0);
    let bases = [Vec2::new(margin, s * 0.5), Vec2::new(s - margin, s * 0.5)];
    let points: Vec<Vec2> = (0..capture_points)
        .map(|i| {
            let x = s * 0.2 + s * 0.6 * (i as f32 + 0.5) / capture_points.max(1) as f32;
            let y = s * 0.5 + rng.range(-0.3, 0.3) * s;
            Vec2::new(x, y)
        })
        .collect();

    // Вокруг баз и точек оставляем свободное место.
    let clear_radius = 14.0;
    let is_clear = |p: Vec2, extra: f32| {
        bases.iter().chain(points.iter()).all(|c| c.distance(p) > clear_radius + extra)
    };

    // Скалы — неразрушаемые пятна.
    for _ in 0..(size * size / 20_000) {
        let c = Vec2::new(rng.range(0.0, s), rng.range(0.0, s));
        let r = rng.range(2.0, 5.0);
        if !is_clear(c, r) {
            continue;
        }
        fill_circle(&mut map, c, r, INDESTRUCTIBLE);
    }

    // Дома: прямоугольник из стен с проёмами-дверями.
    for _ in 0..(size * size / 5_000) {
        let w = rng.range(6.0, 20.0) as i32;
        let h = rng.range(6.0, 16.0) as i32;
        let x0 = rng.range(4.0, s - 4.0 - w as f32) as i32;
        let y0 = rng.range(4.0, s - 4.0 - h as f32) as i32;
        let center = Vec2::new(x0 as f32 + w as f32 * 0.5, y0 as f32 + h as f32 * 0.5);
        if !is_clear(center, (w.max(h) as f32) * 0.75) {
            continue;
        }
        for x in x0..x0 + w {
            map.set(x, y0, HOUSE_WALL);
            map.set(x, y0 + h - 1, HOUSE_WALL);
        }
        for y in y0..y0 + h {
            map.set(x0, y, HOUSE_WALL);
            map.set(x0 + w - 1, y, HOUSE_WALL);
        }
        // По двери на каждую сторону с вероятностью 50%, но хотя бы одна будет.
        let forced = rng.below(4);
        for side in 0..4 {
            if side != forced && !rng.chance(0.5) {
                continue;
            }
            // 0, 1 — верхняя и нижняя стены; 2, 3 — левая и правая.
            let (dx, dy, len) = if side < 2 { (1, 0, w) } else { (0, 1, h) };
            let offset = 2 + rng.below((len - 5).max(1) as u32) as i32;
            let (sx, sy) = match side {
                0 => (x0, y0),
                1 => (x0, y0 + h - 1),
                2 => (x0, y0),
                _ => (x0 + w - 1, y0),
            };
            for k in 0..2 {
                map.set(sx + dx * (offset + k), sy + dy * (offset + k), 0);
            }
        }
    }

    // Заборы — тонкие и слабые, ломаются одной гранатой.
    for _ in 0..(size * size / 8_000) {
        let len = rng.range(6.0, 20.0) as i32;
        let horizontal = rng.chance(0.5);
        let x0 = rng.range(4.0, s - 24.0) as i32;
        let y0 = rng.range(4.0, s - 24.0) as i32;
        let mid = Vec2::new(x0 as f32, y0 as f32);
        if !is_clear(mid, len as f32) {
            continue;
        }
        for k in 0..len {
            let (x, y) = if horizontal { (x0 + k, y0) } else { (x0, y0 + k) };
            if !map.is_solid(x, y) {
                map.set(x, y, FENCE);
            }
        }
    }

    // Край карты.
    for i in 0..n {
        map.set(i, 0, INDESTRUCTIBLE);
        map.set(i, n - 1, INDESTRUCTIBLE);
        map.set(0, i, INDESTRUCTIBLE);
        map.set(n - 1, i, INDESTRUCTIBLE);
    }

    (map, Layout { bases, capture_points: points })
}

fn fill_circle(map: &mut Map, c: Vec2, r: f32, strength: u8) {
    let (x0, y0) = Map::cell_of(c - Vec2::splat(r));
    let (x1, y1) = Map::cell_of(c + Vec2::splat(r));
    for y in y0..=y1 {
        for x in x0..=x1 {
            if Vec2::new(x as f32 + 0.5, y as f32 + 0.5).distance(c) <= r {
                map.set(x, y, strength);
            }
        }
    }
}
