//! Процедурная карта: базы по краям, между ними дорога через все точки захвата,
//! вокруг каждой точки — квартал (улицы сеткой, дома из готовых зданий, площадь),
//! у дороги — хутора, по полям — заборы и скалы.
//!
//! Карту по сети не передают: клиент строит её сам из того же зерна. Поэтому
//! генератор строго детерминированный — только `Rng` от зерна и встроенные здания
//! (`prefab::builtin`), никаких файлов.

use crate::map::{EMPTY, INDESTRUCTIBLE, Map};
use crate::prefab::{self, ASPHALT, FLOOR, Prefab};
use crate::rng::Rng;
use glam::Vec2;

pub const HOUSE_WALL: u8 = prefab::HOUSE;
pub const FENCE: u8 = prefab::FENCE;

/// Покрытие земли: трава, сухая трава, грунт (номера из `GROUND_NAMES`).
const GRASS: u8 = 0;
const DRY: u8 = 1;
const DIRT: u8 = 2;

/// Квартал: шаг улиц, ширина улицы, участок между улицами, м.
const BLOCK: i32 = 18;
const STREET: i32 = 4;
const LOT: i32 = BLOCK - STREET;
/// Площадь у точки захвата и свободное место у баз, м.
const PLAZA: f32 = 7.0;
const BASE_CLEAR: f32 = 16.0;

#[derive(Clone, Debug)]
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
    let houses: Vec<Prefab> = prefab::builtin();
    let pick = |names: &[&str], rng: &mut Rng| -> Prefab {
        let name = names[rng.below(names.len() as u32) as usize];
        let p = houses.iter().find(|p| p.name == name).expect("встроенное здание");
        p.rotated(rng.below(4) as u8)
    };
    // Где строить нельзя: у баз и у точек захвата.
    let keep_clear = |c: Vec2, r: f32| bases.iter().any(|b| b.distance(c) < BASE_CLEAR + r) || points.iter().any(|p| p.distance(c) < PLAZA + r);

    // --- Земля: пятна сухой травы и грунта по плавному шуму.
    for y in 0..n {
        for x in 0..n {
            let dry = value_noise(x as f32 / 9.0, y as f32 / 9.0, seed);
            let dirt = value_noise(x as f32 / 23.0 + 17.0, y as f32 / 23.0 + 5.0, seed ^ 0xD1);
            let i = map.index(x, y).unwrap();
            map.ground[i] = if dirt > 0.68 {
                DIRT
            } else if dry > 0.62 {
                DRY
            } else {
                GRASS
            };
        }
    }

    // --- Главная дорога: от базы к базе через все точки, с изгибами.
    let mut route: Vec<Vec2> = points.clone();
    route.sort_by(|a, b| a.x.total_cmp(&b.x));
    route.insert(0, bases[0]);
    route.push(bases[1]);
    let mut road_points = Vec::new();
    for pair in route.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let bend = (b - a).perp().normalize_or_zero() * rng.range(-0.15, 0.15) * a.distance(b);
        let mid = (a + b) * 0.5 + bend;
        road_points.extend(curve(a, mid, b));
    }
    for w in road_points.windows(2) {
        paint_road(&mut map, w[0], w[1], 2.5, ASPHALT);
    }

    // --- Кварталы вокруг точек: улицы сеткой, на участках — дома.
    let town_radius = (s * 0.07).clamp(22.0, 46.0);
    for &center in &points {
        let k = (town_radius / BLOCK as f32).ceil() as i32;
        let (cx, cy) = (center.x.round() as i32, center.y.round() as i32);
        // Улицы — между участками, в пределах квартала (обрезаны по кругу).
        let edge = town_radius + BLOCK as f32 * 0.5;
        for line in -k - 1..=k {
            let off = (line * BLOCK + BLOCK / 2) as f32;
            if off.abs() >= edge {
                continue;
            }
            let half = (edge * edge - off * off).sqrt();
            let c = Vec2::new(cx as f32, cy as f32);
            paint_road(&mut map, c + Vec2::new(-half, off), c + Vec2::new(half, off), STREET as f32 * 0.5, ASPHALT);
            paint_road(&mut map, c + Vec2::new(off, -half), c + Vec2::new(off, half), STREET as f32 * 0.5, ASPHALT);
        }
        // Площадь у точки.
        paint_disc(&mut map, center, PLAZA + 1.0, FLOOR);
        for j in -k..=k {
            for i in -k..=k {
                let lot = Vec2::new((cx + i * BLOCK) as f32, (cy + j * BLOCK) as f32);
                if lot.distance(center) > town_radius || keep_clear(lot, LOT as f32 * 0.5) {
                    continue;
                }
                let roll = rng.f32();
                let building = if roll < 0.55 {
                    pick(&["Дом малый", "Дом в две комнаты", "Дом угловой"], &mut rng)
                } else if roll < 0.7 {
                    pick(&["Двор с домом", "Бункер"], &mut rng)
                } else if roll < 0.85 {
                    pick(&["Руины", "Сарай"], &mut rng)
                } else {
                    continue;
                };
                let (x0, y0) = (cx + i * BLOCK - LOT / 2, cy + j * BLOCK - LOT / 2);
                if !free(&map, x0, y0, LOT, LOT) {
                    continue;
                }
                // Часть участков — за забором с воротами.
                let fenced = rng.chance(0.3);
                if fenced {
                    fence_lot(&mut map, x0, y0, rng.below(4));
                }
                // Дом, а рядом, если влезет, — сарай или развалины: со случайным сдвигом,
                // не выходя на улицу и не вплотную к забору.
                let inner = if fenced { 1 } else { 0 };
                for (k, building) in [building, pick(&["Сарай", "Руины", "Сарай"], &mut rng)].into_iter().enumerate() {
                    let room = LOT - 2 * inner;
                    if building.width > room || building.height > room || (k == 1 && rng.chance(0.4)) {
                        continue;
                    }
                    for _ in 0..4 {
                        let x = x0 + inner + rng.below((room - building.width) as u32 + 1) as i32;
                        let y = y0 + inner + rng.below((room - building.height) as u32 + 1) as i32;
                        if free(&map, x - 1, y - 1, building.width + 2, building.height + 2) {
                            building.stamp(&mut map, x, y);
                            break;
                        }
                    }
                }
            }
        }
    }

    // --- Хутора у дороги: по два-четыре дома в стороне.
    for _ in 0..(size / 128).max(1) * 3 {
        let along = road_points[rng.below(road_points.len() as u32) as usize];
        let side = Vec2::new(rng.range(-1.0, 1.0), rng.range(-1.0, 1.0)).normalize_or(Vec2::Y);
        let center = along + side * rng.range(14.0, 26.0);
        for _ in 0..2 + rng.below(3) {
            let building = pick(&["Дом малый", "Дом в две комнаты", "Двор с домом", "Сарай", "Магазин", "Руины"], &mut rng);
            let at = center + Vec2::new(rng.range(-14.0, 14.0), rng.range(-14.0, 14.0));
            let (x, y) = (at.x as i32 - building.width / 2, at.y as i32 - building.height / 2);
            let mid = Vec2::new(x as f32 + building.width as f32 * 0.5, y as f32 + building.height as f32 * 0.5);
            if x < 3 || y < 3 || x + building.width > n - 3 || y + building.height > n - 3 || keep_clear(mid, building.width.max(building.height) as f32) {
                continue;
            }
            if free(&map, x - 2, y - 2, building.width + 4, building.height + 4) && !on_road(&map, x, y, building.width, building.height) {
                building.stamp(&mut map, x, y);
            }
        }
    }

    // --- Окраины: заборы полей и скалы — не на дорогах и не у точек.
    for _ in 0..(size * size / 12_000) {
        let len = rng.range(8.0, 24.0) as i32;
        let horizontal = rng.chance(0.5);
        let (x0, y0) = (rng.range(4.0, s - 28.0) as i32, rng.range(4.0, s - 28.0) as i32);
        let (w, h) = if horizontal { (len, 1) } else { (1, len) };
        let mid = Vec2::new(x0 as f32 + w as f32 * 0.5, y0 as f32 + h as f32 * 0.5);
        if keep_clear(mid, len as f32) || !free(&map, x0 - 1, y0 - 1, w + 2, h + 2) || on_road(&map, x0, y0, w, h) {
            continue;
        }
        for k in 0..len {
            let (x, y) = if horizontal { (x0 + k, y0) } else { (x0, y0 + k) };
            map.set(x, y, FENCE);
        }
    }
    for _ in 0..(size * size / 40_000) {
        let c = Vec2::new(rng.range(0.0, s), rng.range(0.0, s));
        let r = rng.range(2.0, 4.5);
        let (x, y) = Map::cell_of(c - Vec2::splat(r));
        let d = (r * 2.0).ceil() as i32 + 1;
        if keep_clear(c, r + 4.0) || !free(&map, x - 2, y - 2, d + 4, d + 4) || on_road(&map, x - 1, y - 1, d + 2, d + 2) {
            continue;
        }
        fill_circle(&mut map, c, r, INDESTRUCTIBLE);
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

/// Забор по краю участка с воротами в 3 м на стороне `gate` (0 — сверху, по часовой).
fn fence_lot(map: &mut Map, x0: i32, y0: i32, gate: u32) {
    let last = LOT - 1;
    let middle = LOT / 2;
    for k in 0..LOT {
        for (side, x, y) in [(0, x0 + k, y0), (1, x0 + last, y0 + k), (2, x0 + k, y0 + last), (3, x0, y0 + k)] {
            if side == gate && (k - middle).abs() <= 1 {
                continue;
            }
            map.set(x, y, FENCE);
        }
    }
}

/// Плавная дуга через три точки (квадратичная кривая Безье), отрезками по ~4 м.
fn curve(a: Vec2, mid: Vec2, b: Vec2) -> Vec<Vec2> {
    let steps = ((a.distance(mid) + mid.distance(b)) / 4.0).ceil().max(1.0) as usize;
    (0..=steps)
        .map(|k| {
            let t = k as f32 / steps as f32;
            a * (1.0 - t) * (1.0 - t) + mid * 2.0 * t * (1.0 - t) + b * t * t
        })
        .collect()
}

/// Дорога шириной `2 · half` от `a` до `b`: только покрытие земли, стены не трогаем.
fn paint_road(map: &mut Map, a: Vec2, b: Vec2, half: f32, ground: u8) {
    let steps = (a.distance(b) / 0.5).ceil().max(1.0) as i32;
    for k in 0..=steps {
        paint_disc(map, a.lerp(b, k as f32 / steps as f32), half, ground);
    }
}

fn paint_disc(map: &mut Map, c: Vec2, r: f32, ground: u8) {
    let (x0, y0) = Map::cell_of(c - Vec2::splat(r));
    let (x1, y1) = Map::cell_of(c + Vec2::splat(r));
    for y in y0..=y1 {
        for x in x0..=x1 {
            if Vec2::new(x as f32 + 0.5, y as f32 + 0.5).distance(c) <= r
                && let Some(i) = map.index(x, y)
            {
                map.ground[i] = ground;
            }
        }
    }
}

/// В прямоугольнике нет стен (и он весь на карте).
fn free(map: &Map, x0: i32, y0: i32, w: i32, h: i32) -> bool {
    (y0..y0 + h).all(|y| (x0..x0 + w).all(|x| map.index(x, y).is_some() && map.solid[map.index(x, y).unwrap()] == EMPTY))
}

/// Прямоугольник задевает дорогу.
fn on_road(map: &Map, x0: i32, y0: i32, w: i32, h: i32) -> bool {
    (y0..y0 + h).any(|y| (x0..x0 + w).any(|x| map.index(x, y).is_some_and(|i| map.ground[i] == ASPHALT)))
}

/// Плавный шум 0..1: случайные значения в узлах сетки, между ними — сглаженно.
fn value_noise(x: f32, y: f32, seed: u64) -> f32 {
    let hash = |ix: i32, iy: i32| {
        let mut h = (ix as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (iy as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ seed;
        h ^= h >> 29;
        h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        (h >> 40) as f32 / (1u64 << 24) as f32
    };
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - ix as f32, y - iy as f32);
    let (ux, uy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let top = hash(ix, iy) + (hash(ix + 1, iy) - hash(ix, iy)) * ux;
    let bottom = hash(ix, iy + 1) + (hash(ix + 1, iy + 1) - hash(ix, iy + 1)) * ux;
    top + (bottom - top) * uy
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nav::{FlowField, UNREACHABLE};
    use std::time::Instant;

    #[test]
    fn same_seed_same_map() {
        let (a, la) = generate(256, 3, 9);
        let (b, lb) = generate(256, 3, 9);
        assert!(a.solid == b.solid && a.ground == b.ground && la.capture_points == lb.capture_points);
        let (c, _) = generate(256, 3, 10);
        assert!(a.solid != c.solid);
    }

    #[test]
    fn everything_is_reachable() {
        for (size, seed) in [(192, 1), (512, 2), (512, 3), (1024, 4)] {
            let (map, layout) = generate(size, 5, seed);
            let mut targets = layout.capture_points.clone();
            targets.extend(layout.bases);
            for &from in &layout.capture_points {
                let mut field = FlowField::new(from);
                field.compute(&map);
                for &to in &targets {
                    let (x, y) = Map::cell_of(to);
                    assert_ne!(field.dist[map.index(x, y).unwrap()], UNREACHABLE, "карта {size}/{seed}: от {from:?} не дойти до {to:?}");
                }
            }
            // Точки и базы — на свободном месте.
            for &p in &targets {
                assert!(!map.circle_hits_solid(p, 4.0), "карта {size}/{seed}: у {p:?} стена");
            }
        }
    }

    #[test]
    fn points_have_cover_around() {
        // Раньше у точек было 14 м чистого поля; теперь вокруг — квартал.
        let (map, layout) = generate(512, 5, 7);
        for &p in &layout.capture_points {
            let (x0, y0) = Map::cell_of(p - Vec2::splat(25.0));
            let mut walls = 0;
            for y in y0..y0 + 50 {
                for x in x0..x0 + 50 {
                    if Vec2::new(x as f32 + 0.5, y as f32 + 0.5).distance(p) < 25.0 && map.is_solid(x, y) {
                        walls += 1;
                    }
                }
            }
            assert!(walls > 60, "у точки {p:?} всего {walls} клеток стен в 25 м");
        }
        // Дороги есть, и через точки.
        let asphalt = map.ground.iter().filter(|&&g| g == ASPHALT).count();
        assert!(asphalt > 4000, "асфальта {asphalt} клеток");
    }

    #[test]
    fn big_map_is_quick() {
        let t = Instant::now();
        generate(1024, 5, 1);
        let ms = t.elapsed().as_secs_f32() * 1000.0;
        println!("карта 1024: {ms:.0} мс");
        assert!(ms < 2000.0);
    }
}
