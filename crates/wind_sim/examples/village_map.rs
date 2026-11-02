//! Собирает пробную карту «деревня» в `data/maps/деревня`: грунтовая улица, дворы с избами,
//! сараями и штакетником, колодцы, берёзы, ельник по краям. Предметы — модели из
//! `data/models/props` (их делают в Blender). Карту потом можно править в редакторе.
//!
//! cargo run --release -p wind_sim --example village_map

use std::f32::consts::{FRAC_PI_2, PI, TAU};
use std::path::Path;
use wind_sim::Vec2;
use wind_sim::map::{INDESTRUCTIBLE, Map};
use wind_sim::mapfile::{MapFile, Prop};
use wind_sim::mapgen::Layout;
use wind_sim::rng::Rng;

const SIZE: u32 = 192;
const DRY: u8 = 1;
const DIRT: u8 = 2;
/// Улица — вдоль x посередине.
const STREET: f32 = 96.0;

fn prop(model: &str, x: f32, y: f32, angle: f32, scale: f32) -> Prop {
    Prop { model: model.into(), pos: (x, y), angle, scale }
}

fn main() {
    let mut map = Map::new(SIZE, SIZE);
    let s = SIZE as f32;
    let mut rng = Rng::new(42);

    // Рельеф: деревня на ровном, к краям — пологие холмы.
    for y in 0..=SIZE as i32 {
        for x in 0..=SIZE as i32 {
            let p = Vec2::new(x as f32, y as f32);
            let edge = ((p.y - STREET).abs() - 40.0).max(0.0) / 56.0;
            let h = edge * edge * 4.0 + ((p.x * 0.045).sin() * (p.y * 0.05).cos()).max(0.0) * 1.5 * edge;
            map.set_corner_height(x, y, h);
        }
    }

    // Земля: трава, сухие пятна, грунтовая улица, вытоптанные дворы.
    for y in 0..SIZE as i32 {
        for x in 0..SIZE as i32 {
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            let i = map.index(x, y).unwrap();
            let wave = ((p.x * 0.07).sin() + (p.y * 0.06).cos() + (p.x * 0.013 + p.y * 0.021).sin()) / 3.0;
            // Улица чуть виляет.
            let street = STREET + (p.x * 0.03).sin() * 2.0;
            map.ground[i] = if (p.y - street).abs() < 2.6 {
                DIRT
            } else if wave > 0.3 {
                DRY
            } else {
                0
            };
        }
    }

    let mut props = Vec::new();
    // Дворы по обе стороны улицы: (x середины участка, сторона: −1 — север, 1 — юг).
    let plots: Vec<(f32, f32)> = [40.0, 66.0, 92.0, 118.0, 144.0].iter().flat_map(|&x| [(x, -1.0), (x + 9.0, 1.0)]).collect();
    for (k, &(x, side)) in plots.iter().enumerate() {
        // Изба окнами на улицу: фасад модели смотрит вдоль +y карты.
        let face = if side < 0.0 { 0.0 } else { PI };
        let jitter = rng.range(-1.5, 1.5);
        props.push(prop("изба", x + jitter, STREET + side * 11.0, face + rng.range(-0.04, 0.04), 1.0));
        // Двор утоптан.
        for yy in 0..14 {
            for xx in -8..8 {
                let (cx, cy) = ((x + xx as f32) as i32, (STREET + side * (6.0 + yy as f32)) as i32);
                if let Some(i) = map.index(cx, cy)
                    && rng.range(0.0, 1.0) < 0.6
                {
                    map.ground[i] = if rng.range(0.0, 1.0) < 0.5 { DIRT } else { DRY };
                }
            }
        }
        // Сарай в глубине участка, боком.
        if k % 3 != 2 {
            props.push(prop("сарай", x + 6.0, STREET + side * 22.0, face + FRAC_PI_2 * side, 1.0));
        }
        // Штакетник вдоль улицы — с калиткой (проём) и заброшенными пролётами.
        let fy = STREET + side * 4.5;
        for seg in 0..4 {
            let fx = x - 6.0 + seg as f32 * 3.0;
            if seg == 2 {
                continue; // калитка
            }
            if rng.range(0.0, 1.0) < 0.12 {
                continue; // пролёта нет
            }
            let fallen = rng.range(0.0, 1.0) < 0.08;
            let angle = if fallen { rng.range(0.3, 0.6) } else { rng.range(-0.03, 0.03) };
            props.push(prop("забор", fx + 1.5, fy, angle + if side > 0.0 { PI } else { 0.0 }, 1.0));
        }
        // Забор между участками — поперёк улицы.
        for seg in 0..5 {
            let fy = STREET + side * (6.0 + seg as f32 * 3.0 + 1.5);
            props.push(prop("забор", x - 7.5, fy, FRAC_PI_2, 1.0));
        }
        // Берёзы у дома.
        for _ in 0..2 {
            let bx = x + rng.range(-7.0, 7.0);
            let by = STREET + side * rng.range(17.0, 28.0);
            props.push(prop("берёза", bx, by, rng.range(0.0, TAU), rng.range(0.85, 1.2)));
        }
    }
    // Колодцы у улицы.
    for x in [53.0, 105.0, 157.0] {
        props.push(prop("колодец", x, STREET - 4.0, rng.range(0.0, TAU), 1.0));
    }
    // Ельник по краям и редкие берёзы в поле.
    let mut placed = 0;
    while placed < 260 {
        let (x, y) = (rng.range(6.0, s - 6.0), rng.range(6.0, s - 6.0));
        let from_street = (y - STREET).abs();
        if from_street < 44.0 {
            continue;
        }
        let density = ((from_street - 44.0) / 30.0).min(1.0);
        if rng.range(0.0, 1.0) > density {
            continue;
        }
        let model = if rng.range(0.0, 1.0) < 0.75 { "ель" } else { "берёза" };
        props.push(prop(model, x, y, rng.range(0.0, TAU), rng.range(0.8, 1.35)));
        placed += 1;
    }

    // Край карты.
    let n = SIZE as i32;
    for i in 0..n {
        for (x, y) in [(i, 0), (i, n - 1), (0, i), (n - 1, i)] {
            map.set(x, y, INDESTRUCTIBLE);
        }
    }

    let file = MapFile {
        name: "деревня".into(),
        map,
        layout: Layout {
            bases: [Vec2::new(14.0, STREET), Vec2::new(s - 14.0, STREET)],
            capture_points: vec![Vec2::new(s * 0.5, STREET), Vec2::new(60.0, STREET - 30.0), Vec2::new(132.0, STREET + 30.0)],
        },
        atmosphere: Default::default(),
        props,
    };
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/maps/деревня");
    file.save(&dir).expect("не удалось сохранить");
    println!("Сохранено: {}, предметов {}", dir.display(), file.props.len());
}
