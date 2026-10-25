//! Собирает пример карты «пример» в `data/maps/пример`: холмы, дороги, песок,
//! деревня из домов с заборами, пять точек захвата. Карту потом можно править в редакторе.
//!
//! cargo run --release -p wind_sim --example sample_map

use std::path::Path;
use wind_sim::Vec2;
use wind_sim::map::{INDESTRUCTIBLE, Map};
use wind_sim::mapfile::MapFile;
use wind_sim::mapgen::Layout;

const SIZE: u32 = 384;
const HOUSE: u8 = 120;
const FENCE: u8 = 40;
const DRY: u8 = 1;
const DIRT: u8 = 2;
const SAND: u8 = 3;
const ASPHALT: u8 = 4;
const CONCRETE: u8 = 5;

fn main() {
    let mut map = Map::new(SIZE, SIZE);
    let s = SIZE as f32;

    // Рельеф: несколько мягких холмов.
    let hills = [(90.0, 80.0, 40.0, 7.0), (300.0, 290.0, 55.0, 10.0), (200.0, 330.0, 30.0, 4.0), (310.0, 70.0, 35.0, 5.0)];
    for y in 0..=SIZE as i32 {
        for x in 0..=SIZE as i32 {
            let p = Vec2::new(x as f32, y as f32);
            let h: f32 = hills.iter().map(|&(cx, cy, r, top)| top * (-(p.distance(Vec2::new(cx, cy)) / r).powi(2)).exp()).sum();
            map.set_corner_height(x, y, h);
        }
    }

    // Покрытие: сухая трава пятнами, песчаный карьер, земля у холмов.
    for y in 0..SIZE as i32 {
        for x in 0..SIZE as i32 {
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            let i = map.index(x, y).unwrap();
            let wave = ((p.x * 0.05).sin() + (p.y * 0.043).cos()) * 0.5;
            map.ground[i] = if p.distance(Vec2::new(300.0, 80.0)) < 38.0 {
                SAND
            } else if map.height_at(p) > 4.5 {
                DIRT
            } else if wave > 0.55 {
                DRY
            } else {
                0
            };
        }
    }

    // Дороги: крест через центр и съезд к карьеру.
    let mut road = |a: Vec2, b: Vec2, width: f32| {
        let len = a.distance(b);
        let steps = (len * 2.0) as i32;
        for k in 0..=steps {
            let c = a.lerp(b, k as f32 / steps as f32);
            for dy in -(width as i32)..=width as i32 {
                for dx in -(width as i32)..=width as i32 {
                    let p = c + Vec2::new(dx as f32, dy as f32);
                    if p.distance(c) <= width
                        && let Some(i) = map.index(p.x as i32, p.y as i32)
                    {
                        map.ground[i] = ASPHALT;
                    }
                }
            }
        }
    };
    road(Vec2::new(20.0, s * 0.5), Vec2::new(s - 20.0, s * 0.5), 3.0);
    road(Vec2::new(s * 0.5, 20.0), Vec2::new(s * 0.5, s - 20.0), 3.0);
    road(Vec2::new(s * 0.5, s * 0.5), Vec2::new(300.0, 80.0), 2.0);

    // Деревня у перекрёстка: дома с дверями, бетонные дворы, заборы.
    let house = |map: &mut Map, x0: i32, y0: i32, w: i32, h: i32| {
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                let i = map.index(x, y).unwrap();
                map.ground[i] = CONCRETE;
            }
        }
        for x in x0..x0 + w {
            for y in [y0, y0 + h - 1] {
                if x != x0 + w / 2 && x != x0 + w / 2 + 1 {
                    map.set(x, y, HOUSE);
                }
            }
        }
        for y in y0..y0 + h {
            for x in [x0, x0 + w - 1] {
                map.set(x, y, HOUSE);
            }
        }
    };
    let c = (s * 0.5) as i32;
    for &(dx, dy, w, h) in &[(-40, -30, 14, 10), (-22, -32, 10, 12), (12, -34, 16, 12), (34, -28, 10, 9), (-38, 14, 12, 12), (-18, 18, 14, 10), (14, 16, 12, 14), (32, 20, 14, 10)] {
        house(&mut map, c + dx, c + dy, w, h);
    }
    let mut fence = |a: (i32, i32), b: (i32, i32)| {
        let (mut x, mut y) = a;
        while (x, y) != b {
            map.set(x, y, FENCE);
            x += (b.0 - x).signum();
            y += (b.1 - y).signum();
        }
    };
    fence((c - 46, c - 40), (c - 6, c - 40));
    fence((c + 8, c - 40), (c + 48, c - 40));
    fence((c - 46, c + 36), (c - 6, c + 36));
    fence((c + 8, c + 38), (c + 50, c + 38));

    // Скалы на вершине большого холма и край карты.
    for (cx, cy, r) in [(300.0, 290.0, 4.0), (290.0, 300.0, 3.0), (90.0, 80.0, 3.0)] {
        for y in (cy - r) as i32..=(cy + r) as i32 {
            for x in (cx - r) as i32..=(cx + r) as i32 {
                if Vec2::new(x as f32, y as f32).distance(Vec2::new(cx, cy)) <= r {
                    map.set(x, y, INDESTRUCTIBLE);
                }
            }
        }
    }
    let n = SIZE as i32;
    for i in 0..n {
        for (x, y) in [(i, 0), (i, n - 1), (0, i), (n - 1, i)] {
            map.set(x, y, INDESTRUCTIBLE);
        }
    }

    let file = MapFile {
        name: "пример".into(),
        map,
        layout: Layout {
            bases: [Vec2::new(24.0, s * 0.5 + 12.0), Vec2::new(s - 24.0, s * 0.5 - 12.0)],
            capture_points: vec![
                Vec2::new(s * 0.5, s * 0.5),
                Vec2::new(95.0, 95.0),
                Vec2::new(285.0, 275.0),
                Vec2::new(300.0, 100.0),
                Vec2::new(110.0, 300.0),
            ],
        },
    };
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/maps/пример");
    file.save(&dir).expect("не удалось сохранить");
    println!("Сохранено: {}", dir.display());
}
