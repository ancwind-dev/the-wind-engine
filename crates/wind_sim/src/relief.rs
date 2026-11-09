//! Рельеф: пологие холмы и гряды поверх карты (до ~7 м).
//!
//! Холмы — плавный шум в три слоя (крупные холмы ~60 м, гряды ~25 м, бугры ~10 м),
//! с равнинами между ними. Там, где должно быть ровно, холмы не мешают:
//! - здания (стены `HOUSE`, `CONCRETE`, полы `FLOOR`) и места из `keep` (точки захвата,
//!   базы, предметы) — ровные площадки: у каждой своя высота, средняя по сильно
//!   сглаженному («спокойному») рельефу под ней;
//! - дороги (`ASPHALT`) идут по спокойному рельефу — плавно вверх и вниз;
//! - у воды холмов нет вовсе: озёра и берег остаются как были.
//!
//! Между ровным и холмами — плавный переход на ~8 м. Холмы только прибавляются к высоте:
//! ниже уровня воды суша не опускается, новых луж не появляется.
//!
//! Всё — от зерна, без случайности запуска: процедурную карту клиент строит сам и должен
//! получить те же холмы, что и сервер.

use crate::map::Map;
use crate::prefab::{ASPHALT, CONCRETE, FLOOR, HOUSE};
use glam::Vec2;

/// Высота холмов по умолчанию, м: «пологие, до 6–8 м».
pub const HILLS: f32 = 7.0;

/// Насколько сильно сглажена «спокойная» высота под домами и дорогами (радиус размытия, м).
const CALM: usize = 16;
/// На сколько клеток размазывается поправка площадок (радиус размытия, дважды).
const PAD_SPREAD: usize = 4;
/// Переход от ровного к холмам, м: до `FLAT_NEAR` — ровно, к `FLAT_FAR` — холм целиком.
const FLAT_NEAR: f32 = 1.5;
const FLAT_FAR: f32 = 9.0;
/// У воды: до `SHORE_NEAR` холмов нет, к `SHORE_FAR` — целиком.
const SHORE_NEAR: f32 = 3.0;
const SHORE_FAR: f32 = 12.0;

/// Поднять холмы на карте. `height` — высота самых высоких холмов, м; `keep` — круги
/// (центр, радиус), где должно быть ровно.
pub fn raise_hills(map: &mut Map, seed: u64, height: f32, keep: &[(Vec2, f32)]) {
    let (w, h) = (map.width as usize, map.height as usize);
    let (cw, ch) = (w + 1, h + 1);

    // Холмы в углах клеток.
    let mut raw = vec![0f32; cw * ch];
    for y in 0..ch {
        for x in 0..cw {
            raw[y * cw + x] = height * hill_shape(x as f32, y as f32, seed);
        }
    }
    // «Спокойная» высота: дважды размытая — почти гауссово.
    let calm = blur(&blur(&raw, cw, ch, CALM), cw, ch, CALM);

    // Где ровно и где вода — по клеткам; расстояние до них — по клеткам, м.
    // «Площадки» — здания (стены и полы) и места из `keep`: у каждой связной площадки одна
    // высота, средняя по спокойному рельефу под ней, — дом стоит ровно. Дороги не площадки:
    // они идут по спокойной высоте, плавно вверх и вниз.
    let water = map.water;
    let mut flat = vec![false; w * h];
    let mut pad = vec![false; w * h];
    let mut wet = vec![false; w * h];
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let c = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            pad[i] = matches!(map.solid[i], HOUSE | CONCRETE) || map.ground[i] == FLOOR || keep.iter().any(|&(k, r)| k.distance(c) <= r);
            flat[i] = pad[i] || map.ground[i] == ASPHALT;
            wet[i] = water.is_some_and(|lvl| map.height_at(c) < lvl + 0.15);
        }
    }
    let (labels, pad_height) = pads(&pad, w, h, &calm, cw);
    let to_pad = distance(&pad, w, h);
    let to_flat = distance(&flat, w, h);
    let to_water = distance(&wet, w, h);

    // Поправка площадок к спокойной высоте: в углах у площадок — точная (высота площадки
    // минус спокойная), дальше — размазанная по соседям (нормированное размытие). Так
    // между двумя площадками разной высоты нет ступеньки там, где «ближняя» сменяется.
    let mut pad_off = vec![0f32; cw * ch];
    let mut pad_w = vec![0f32; cw * ch];
    let mut on_pad = vec![false; cw * ch];
    for y in 0..ch {
        for x in 0..cw {
            let i = y * cw + x;
            let (mut sum, mut n) = (0.0, 0.0);
            for (cx, cy) in [(x.wrapping_sub(1), y.wrapping_sub(1)), (x, y.wrapping_sub(1)), (x.wrapping_sub(1), y), (x, y)] {
                if cx < w && cy < h && labels[cy * w + cx] != u32::MAX {
                    sum += pad_height[labels[cy * w + cx] as usize] - calm[i];
                    n += 1.0;
                }
            }
            if n > 0.0 {
                on_pad[i] = true;
                pad_off[i] = sum / n;
                pad_w[i] = 1.0;
            }
        }
    }
    let spread = |v: &[f32]| blur(&blur(v, cw, ch, PAD_SPREAD), cw, ch, PAD_SPREAD);
    let (off_sum, off_w) = (spread(&pad_off), spread(&pad_w));

    for y in 0..ch {
        for x in 0..cw {
            // Угол делят до четырёх клеток — берём ближайшую.
            let mut d = [f32::MAX; 3];
            for (cx, cy) in [(x.wrapping_sub(1), y.wrapping_sub(1)), (x, y.wrapping_sub(1)), (x.wrapping_sub(1), y), (x, y)] {
                if cx < w && cy < h {
                    let j = cy * w + cx;
                    d[0] = d[0].min(to_flat[j]);
                    d[1] = d[1].min(to_water[j]);
                    d[2] = d[2].min(to_pad[j]);
                }
            }
            let i = y * cw + x;
            // Основа: на площадке — её высота, рядом — спокойный рельеф с поправкой
            // площадок, затухающей к `FLAT_FAR`.
            let base = if on_pad[i] {
                calm[i] + pad_off[i]
            } else if off_w[i] > 1e-4 {
                calm[i] + off_sum[i] / off_w[i] * (1.0 - smoothstep(FLAT_NEAR, FLAT_FAR, d[2]))
            } else {
                calm[i]
            };
            let k_flat = smoothstep(FLAT_NEAR, FLAT_FAR, d[0]);
            let k_shore = smoothstep(SHORE_NEAR, SHORE_FAR, d[1]);
            let add = (base + (raw[i] - base) * k_flat) * k_shore;
            let (xi, yi) = (x as i32, y as i32);
            let old = map.corner_height(xi, yi);
            map.set_corner_height(xi, yi, old + add.max(0.0));
        }
    }
    for c in &mut map.chunk_revisions {
        *c += 1;
    }
    map.revision += 1;
}

/// Форма холмов в точке (0..1): три слоя шума и равнины между холмами.
fn hill_shape(x: f32, y: f32, seed: u64) -> f32 {
    let n = noise(x / 64.0, y / 64.0, seed ^ 0x4111) * 0.58
        + noise(x / 26.0 + 7.3, y / 26.0 + 1.9, seed ^ 0x4112) * 0.29
        + noise(x / 10.5 + 3.1, y / 10.5 + 8.7, seed ^ 0x4113) * 0.13;
    smoothstep(0.36, 0.8, n)
}

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Плавный шум 0..1: значения в узлах сетки — от хеша, между ними — сглаженно.
fn noise(x: f32, y: f32, seed: u64) -> f32 {
    let hash = |ix: i32, iy: i32| {
        let mut h = (ix as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (iy as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F) ^ seed;
        h ^= h >> 29;
        h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 32;
        (h >> 40) as f32 / (1u64 << 24) as f32
    };
    let (ix, iy) = (x.floor() as i32, y.floor() as i32);
    let (fx, fy) = (x - ix as f32, y - iy as f32);
    let (ux, uy) = (fx * fx * (3.0 - 2.0 * fx), fy * fy * (3.0 - 2.0 * fy));
    let top = hash(ix, iy) + (hash(ix + 1, iy) - hash(ix, iy)) * ux;
    let bottom = hash(ix, iy + 1) + (hash(ix + 1, iy + 1) - hash(ix, iy + 1)) * ux;
    top + (bottom - top) * uy
}

/// Размытие квадратом радиуса `r` (по строкам, потом по столбцам), края — повтором.
fn blur(src: &[f32], w: usize, h: usize, r: usize) -> Vec<f32> {
    let pass = |src: &[f32], len: usize, lines: usize, at: &dyn Fn(usize, usize) -> usize| {
        let mut out = vec![0f32; src.len()];
        let n = (2 * r + 1) as f32;
        for line in 0..lines {
            let get = |k: isize| src[at(line, k.clamp(0, len as isize - 1) as usize)];
            let mut sum: f32 = (-(r as isize)..=r as isize).map(get).sum();
            for k in 0..len {
                out[at(line, k)] = sum / n;
                sum += get(k as isize + r as isize + 1) - get(k as isize - r as isize);
            }
        }
        out
    };
    let rows = pass(src, w, h, &|line, k| line * w + k);
    pass(&rows, h, w, &|line, k| k * w + line)
}

/// Связные площадки (8 соседей): номер площадки у каждой клетки (`u32::MAX` — не
/// площадка) и высота каждой — средняя `calm` под её клетками.
fn pads(pad: &[bool], w: usize, h: usize, calm: &[f32], cw: usize) -> (Vec<u32>, Vec<f32>) {
    let mut label = vec![u32::MAX; w * h];
    let mut heights = Vec::new();
    let mut stack = Vec::new();
    for start in 0..w * h {
        if !pad[start] || label[start] != u32::MAX {
            continue;
        }
        let id = heights.len() as u32;
        let (mut sum, mut n) = (0.0f64, 0u32);
        label[start] = id;
        stack.push(start);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            sum += calm[y * cw + x] as f64;
            n += 1;
            for dy in -1i32..=1 {
                for dx in -1i32..=1 {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        continue;
                    }
                    let j = ny as usize * w + nx as usize;
                    if pad[j] && label[j] == u32::MAX {
                        label[j] = id;
                        stack.push(j);
                    }
                }
            }
        }
        heights.push((sum / n as f64) as f32);
    }
    (label, heights)
}

/// Расстояние от каждой клетки до ближайшей отмеченной, м (по клеткам, 8 соседей —
/// 1 и √2; два прохода). Нет отмеченных — бесконечность.
fn distance(marked: &[bool], w: usize, h: usize) -> Vec<f32> {
    let mut d: Vec<f32> = marked.iter().map(|&m| if m { 0.0 } else { f32::MAX / 4.0 }).collect();
    let diag = std::f32::consts::SQRT_2;
    let steps: [(isize, isize, f32); 4] = [(-1, 0, 1.0), (0, -1, 1.0), (-1, -1, diag), (1, -1, diag)];
    for pass in 0..2 {
        for k in 0..w * h {
            let i = if pass == 0 { k } else { w * h - 1 - k };
            let (x, y) = ((i % w) as isize, (i / w) as isize);
            let mut v = d[i];
            for &(dx, dy, step) in &steps {
                // Второй проход — зеркально: соседи справа и снизу.
                let (dx, dy) = if pass == 0 { (dx, dy) } else { (-dx, -dy) };
                let (nx, ny) = (x + dx, y + dy);
                if nx >= 0 && ny >= 0 && (nx as usize) < w && (ny as usize) < h {
                    v = v.min(d[ny as usize * w + nx as usize] + step);
                }
            }
            d[i] = v;
        }
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Поле 160×160 с домом (стены `HOUSE`), дорогой поперёк и озером.
    fn sample() -> Map {
        let mut map = Map::new(160, 160);
        for y in 70..80 {
            for x in 0..160 {
                map.ground[y * 160 + x] = ASPHALT;
            }
        }
        for y in 20..30 {
            for x in 20..30 {
                if x == 20 || x == 29 || y == 20 || y == 29 {
                    map.set(x, y, HOUSE);
                }
            }
        }
        // Озеро: суша на 3 м, яма в середине ниже воды.
        for y in 0..=160 {
            for x in 0..=160 {
                let d = Vec2::new(x as f32, y as f32).distance(Vec2::new(120.0, 120.0));
                map.set_corner_height(x, y, if d < 15.0 { 1.0 } else { 3.0 });
            }
        }
        map.water = Some(2.6);
        map
    }

    #[test]
    fn hills_rise_but_roads_houses_and_lakes_stay_calm() {
        let mut map = sample();
        let before = map.heights.clone();
        raise_hills(&mut map, 7, HILLS, &[(Vec2::new(60.0, 120.0), 6.0)]);
        let h = |x: i32, y: i32| map.corner_height(x, y);
        // Холмы есть и не выше заданного.
        let rise: Vec<f32> = map.heights.iter().zip(&before).map(|(a, b)| (*a as f32 - *b as f32) / crate::map::HEIGHT_SCALE).collect();
        let top = rise.iter().copied().fold(0.0, f32::max);
        assert!(top > 3.0 && top <= HILLS + 0.01, "высота холмов {top}");
        assert!(rise.iter().all(|&r| r >= 0.0), "холмы только прибавляются");
        // Дом — на почти ровной площадке: перепад по углам стен меньше 0,6 м.
        let corners: Vec<f32> = (20..=30).flat_map(|x| [h(x, 20), h(x, 30)]).collect();
        let spread = corners.iter().copied().fold(f32::MIN, f32::max) - corners.iter().copied().fold(f32::MAX, f32::min);
        assert!(spread < 0.6, "под домом перепад {spread}");
        // Дорога — без крутых бугров: соседние углы вдоль неё отличаются не больше чем на 0,15 м.
        for x in 0..160 {
            assert!((h(x + 1, 75) - h(x, 75)).abs() < 0.15, "дорога крутая у x = {x}");
        }
        // Озеро и берег — как были.
        assert_eq!(h(120, 120), 1.0);
        assert!((h(120, 137) - 3.0).abs() < 0.01, "берег поднялся до {}", h(120, 137));
        // Место из `keep` — тоже спокойно.
        assert!((h(61, 120) - h(59, 120)).abs() < 0.2);
    }

    #[test]
    fn no_steps_between_pads() {
        let mut map = Map::new(160, 160);
        // Два дома с улицей между ними, на склоне холма.
        for (x0, y0) in [(40, 40), (40, 56)] {
            for y in y0..y0 + 10 {
                for x in x0..x0 + 10 {
                    if x == x0 || x == x0 + 9 || y == y0 || y == y0 + 9 {
                        map.set(x, y, HOUSE);
                    }
                }
            }
        }
        for seed in 0..12 {
            let mut m = map.clone();
            raise_hills(&mut m, seed, HILLS, &[]);
            for y in 0..160 {
                for x in 0..160 {
                    let (a, b, c) = (m.corner_height(x, y), m.corner_height(x + 1, y), m.corner_height(x, y + 1));
                    assert!((a - b).abs() < 0.9 && (a - c).abs() < 0.9, "зерно {seed}: ступенька у ({x}, {y}): {a} / {b} / {c}");
                }
            }
        }
    }

    #[test]
    fn same_seed_same_hills() {
        let (mut a, mut b) = (sample(), sample());
        raise_hills(&mut a, 3, HILLS, &[]);
        raise_hills(&mut b, 3, HILLS, &[]);
        assert_eq!(a.heights, b.heights);
        let mut c = sample();
        raise_hills(&mut c, 4, HILLS, &[]);
        assert_ne!(a.heights, c.heights);
    }
}
