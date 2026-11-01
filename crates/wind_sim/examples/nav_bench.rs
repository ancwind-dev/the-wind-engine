//! Замер поиска пути на больших картах: поле по всей карте (как было) против графа
//! чанков с полями в чанках по требованию.
//!
//! cargo run --release -p wind_sim --example nav_bench

use std::time::Instant;
use wind_sim::map::EMPTY;
use wind_sim::mapgen::generate;
use wind_sim::nav::{FlowField, NavGraph, distances};
use wind_sim::rng::Rng;

fn ms(t: Instant) -> f32 {
    t.elapsed().as_secs_f32() * 1000.0
}

fn main() {
    for size in [1024, 2048, 4096] {
        let t = Instant::now();
        let (mut map, layout) = generate(size, 5, 1);
        println!("Карта {size} м: создана за {:.0} мс, целей {}", ms(t), layout.capture_points.len() + 2);
        let target = layout.capture_points[0];

        let t = Instant::now();
        let full = distances(&map, target);
        println!("  поле по всей карте (как было): {:.1} мс и {:.0} МБ на цель", ms(t), (full.len() * 4) as f32 / 1e6);

        let t = Instant::now();
        let mut nav = NavGraph::build(&map);
        println!("  граф чанков: {:.1} мс, узлов {}", ms(t), nav.node_count());
        let t = Instant::now();
        let mut field = FlowField::new(target);
        field.compute(&nav, &map);
        println!("  поле цели по графу: {:.2} мс", ms(t));

        // Поля в чанках — там, куда заглянули: по случайным клеткам.
        let mut rng = Rng::new(7);
        let t = Instant::now();
        let n = 300;
        for _ in 0..n {
            let (x, y) = (rng.range(0.0, size as f32) as i32, rng.range(0.0, size as f32) as i32);
            field.distance(&nav, &map, x, y);
        }
        println!("  поле в чанке: {:.3} мс (4 КБ)", ms(t) / n as f32);

        // Пролом в стене: пересчитать граф и поле цели.
        if let Some(i) = map.solid.iter().position(|&s| s != EMPTY && s != 255) {
            let (x, y) = ((i % size as usize) as i32, (i / size as usize) as i32);
            map.solid[i] = EMPTY;
            map.touch(x, y, true);
        }
        let t = Instant::now();
        nav.refresh(&map);
        let graph = ms(t);
        let t = Instant::now();
        field.compute(&nav, &map);
        println!("  после пролома: граф {graph:.2} мс, поле цели {:.2} мс", ms(t));
    }
}
