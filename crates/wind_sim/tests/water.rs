//! Вода: озёра на процедурной карте, вброд и вплавь, ИИ обходит глубину.

use wind_sim::map::{Map, SWIM_DEPTH};
use wind_sim::{Event, PlayerInput, SimConfig, Vec2, World};

fn world(seed: u64) -> World {
    let mut c = SimConfig { map_size: 256, soldiers_per_team: 6, capture_points: 3, ..Default::default() };
    c.round.hold_time = f32::INFINITY;
    c.round.time_limit = 0.0;
    World::new(c, seed)
}

/// Самая глубокая клетка карты.
fn deepest(map: &Map) -> Vec2 {
    let mut best = (0.0, Vec2::ZERO);
    for y in 0..map.height as i32 {
        for x in 0..map.width as i32 {
            let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            let d = map.water_depth(p);
            if d > best.0 {
                best = (d, p);
            }
        }
    }
    best.1
}

#[test]
fn generated_map_has_a_lake_away_from_bases_and_points() {
    let w = world(5);
    assert!(w.map.water.is_some(), "на карте 256 — озеро");
    let lake = deepest(&w.map);
    let depth = w.map.water_depth(lake);
    println!("озеро: середина {lake:?}, глубина {depth:.2} м");
    assert!(depth > SWIM_DEPTH, "в середине — вплавь");
    for p in w.bases.iter().chain(w.points.iter().map(|p| &p.pos)) {
        assert_eq!(w.map.water_depth(*p), 0.0, "база или точка в воде: {p:?}");
    }
    // В озере нет стен.
    for y in 0..w.map.height as i32 {
        for x in 0..w.map.width as i32 {
            if w.map.water_depth(Vec2::new(x as f32 + 0.5, y as f32 + 0.5)) > 0.0 {
                assert!(!w.map.is_solid(x, y) || x == 0 || y == 0, "стена в воде ({x}, {y})");
            }
        }
    }
}

/// Сколько метров за 2 с пройдёт игрок, идущий на восток из `from`.
fn walk_from(w: &mut World, me: u32, from: Vec2) -> f32 {
    w.soldiers[me as usize].pos = from;
    for _ in 0..40 {
        let pos = w.soldiers[me as usize].pos;
        w.set_input(me, PlayerInput { move_dir: Vec2::X, aim: pos + Vec2::X, ..Default::default() });
        w.step();
    }
    w.soldiers[me as usize].pos.distance(from)
}

#[test]
fn water_slows_and_swimmers_do_not_shoot() {
    let mut w = world(5);
    let me = w.take_control(0).unwrap();
    let lake = deepest(&w.map);
    let dry = w.bases[0] + Vec2::new(0.0, 6.0);
    let on_land = walk_from(&mut w, me, dry);
    let swimming = walk_from(&mut w, me, lake - Vec2::new(4.0, 0.0));
    println!("за 2 с: по суше {on_land:.1} м, вплавь {swimming:.1} м");
    assert!(swimming < on_land * 0.55, "вплавь медленнее");
    // Вплавь не стреляет, на суше — стреляет.
    let shoots_at = |w: &mut World, at: Vec2| {
        w.soldiers[me as usize].pos = at;
        w.soldiers[me as usize].cooldown = 0.0;
        w.soldiers[me as usize].reload_timer = 0.0;
        w.set_input(me, PlayerInput { aim: at + Vec2::new(0.0, 10.0), fire: true, ..Default::default() });
        w.step();
        w.events.iter().any(|e| matches!(e, Event::Shot { shooter, .. } if *shooter == me))
    };
    assert!(!shoots_at(&mut w, lake));
    assert!(shoots_at(&mut w, dry));
}

#[test]
fn ai_paths_go_around_deep_water() {
    let w = world(5);
    let lake = deepest(&w.map);
    // С берега и вокруг озера путь к любой точке не ведёт в глубокую воду.
    let mut checked = 0;
    for field in &w.flow {
        for a in 0..32 {
            let from = lake + Vec2::from_angle(a as f32 / 32.0 * std::f32::consts::TAU) * 26.0;
            if w.map.is_solid_at(from) || w.map.water_depth(from) > 0.0 {
                continue;
            }
            if let Some(dir) = field.direction(&w.nav, &w.map, from) {
                let next = from + dir * 1.0;
                let (x, y) = Map::cell_of(next);
                assert!(!w.map.is_deep(x, y), "путь ведёт в глубину из {from:?}");
                checked += 1;
            }
        }
    }
    assert!(checked > 10);
}

#[test]
fn deep_mask_matches_is_deep() {
    let w = world(5);
    let mask = w.map.deep_mask();
    assert!(mask.iter().any(|&d| d), "на карте нет глубокой воды");
    for y in 0..w.map.height as i32 {
        for x in 0..w.map.width as i32 {
            assert_eq!(mask[(y * w.map.width as i32 + x) as usize], w.map.is_deep(x, y), "клетка {x},{y}");
        }
    }
}
