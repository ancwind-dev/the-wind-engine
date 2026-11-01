//! Пути через граф чанков: доходят до цели, почти не длиннее точных и знают о проломах.

use wind_sim::Vec2;
use wind_sim::map::{EMPTY, Map};
use wind_sim::mapgen::generate;
use wind_sim::nav::{FlowField, NavGraph, UNREACHABLE, distances};
use wind_sim::rng::Rng;

/// Идём по полю из клетки `from` по шагу в клетку; цена пути (шаг — 5, по диагонали — 7)
/// или `None`, если застряли.
fn walk(nav: &NavGraph, map: &Map, field: &FlowField, from: (i32, i32)) -> Option<u32> {
    let mut cell = from;
    let mut cost = 0;
    for _ in 0..map.width * 4 {
        if field.distance(nav, map, cell.0, cell.1) == 0 {
            return Some(cost);
        }
        let pos = Vec2::new(cell.0 as f32 + 0.5, cell.1 as f32 + 0.5);
        let dir = field.direction(nav, map, pos)?;
        let next = Map::cell_of(pos + dir);
        if next == cell {
            // У самой цели направление — прямо на неё, внутри клетки.
            return Some(cost);
        }
        cost += if next.0 != cell.0 && next.1 != cell.1 { 7 } else { 5 };
        cell = next;
    }
    None
}

/// Сравнить пути по полю с точными на карте: (средняя длина к точной, худшая, застряли).
fn compare(map: &Map, nav: &NavGraph, target: Vec2, starts: usize, seed: u64) -> (f32, f32, usize) {
    let mut field = FlowField::new(target);
    field.compute(nav, map);
    let exact = distances(map, target);
    let mut rng = Rng::new(seed);
    let (mut sum, mut worst, mut stuck, mut n) = (0.0, 1.0f32, 0, 0);
    while n < starts {
        let (x, y) = (rng.range(0.0, map.width as f32) as i32, rng.range(0.0, map.height as f32) as i32);
        let best = exact[map.index(x, y).unwrap()];
        if best == UNREACHABLE || best < 100 {
            continue;
        }
        n += 1;
        assert_ne!(field.distance(nav, map, x, y), UNREACHABLE, "граф не знает пути из ({x}, {y})");
        match walk(nav, map, &field, (x, y)) {
            Some(cost) => {
                let ratio = cost as f32 / best as f32;
                sum += ratio;
                worst = worst.max(ratio);
            }
            None => stuck += 1,
        }
    }
    (sum / starts as f32, worst, stuck)
}

#[test]
fn paths_on_generated_maps_reach_the_target_and_are_nearly_shortest() {
    for (size, seed) in [(256, 5), (512, 2), (1024, 4)] {
        let (map, layout) = generate(size, 5, seed);
        let nav = NavGraph::build(&map);
        let mut targets = layout.capture_points.clone();
        targets.extend(layout.bases);
        for (k, &t) in targets.iter().enumerate() {
            let (avg, worst, stuck) = compare(&map, &nav, t, 150, seed * 10 + k as u64);
            println!("карта {size}/{seed}, цель {k}: длиннее точного в среднем ×{avg:.3}, худший ×{worst:.2}, застряли {stuck}");
            assert_eq!(stuck, 0, "карта {size}/{seed}: застряли по пути к {t:?}");
            assert!(avg < 1.08, "карта {size}/{seed}: пути в среднем на {:.0} % длиннее", (avg - 1.0) * 100.0);
            assert!(worst < 1.5, "карта {size}/{seed}: худший путь ×{worst:.2}");
        }
    }
}

#[test]
fn across_an_open_field_the_path_is_straight() {
    let map = Map::new(512, 512);
    let nav = NavGraph::build(&map);
    let target = Vec2::new(430.5, 470.5);
    let mut field = FlowField::new(target);
    field.compute(&nav, &map);
    let exact = distances(&map, target);
    for from in [(20, 30), (480, 10), (5, 500), (250, 260)] {
        let cost = walk(&nav, &map, &field, from).expect("застрял в чистом поле");
        let best = exact[map.index(from.0, from.1).unwrap()];
        println!("из {from:?}: ×{:.3}", cost as f32 / best as f32);
        assert!(cost as f32 <= best as f32 * 1.03, "из {from:?}: путь {cost} против {best}");
    }
}

#[test]
fn a_breach_in_the_wall_opens_a_shorter_way() {
    // Стена поперёк карты с проходом только у края.
    let mut map = Map::new(256, 256);
    for y in 10..256 {
        let i = map.index(128, y).unwrap();
        map.solid[i] = 255;
    }
    let mut nav = NavGraph::build(&map);
    let target = Vec2::new(200.5, 200.5);
    let mut field = FlowField::new(target);
    field.compute(&nav, &map);
    let from = (60, 200);
    let around = field.distance(&nav, &map, from.0, from.1);
    assert_ne!(around, UNREACHABLE);
    let walked = walk(&nav, &map, &field, from).expect("в обход стены не дошли");
    println!("в обход: по полю {around}, прошли {walked}");
    assert!(walked as f32 <= around as f32 * 1.02);

    // Пробили стену напротив.
    for y in 198..203 {
        let i = map.index(128, y).unwrap();
        map.solid[i] = EMPTY;
        map.touch(128, y, true);
    }
    assert!(nav.refresh(&map));
    assert!(field.is_stale(&nav));
    field.compute(&nav, &map);
    let through = field.distance(&nav, &map, from.0, from.1);
    println!("в обход {around}, через пролом {through}");
    assert!(through < around / 2, "пролом не сократил путь: {through} против {around}");
    let exact = distances(&map, target)[map.index(from.0, from.1).unwrap()];
    assert!(through as f32 <= exact as f32 * 1.03);
}

