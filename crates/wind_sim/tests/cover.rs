//! Укрытия в бою: солдаты стреляют из-за углов, прячутся на перезарядке.

use wind_sim::atmosphere::Atmosphere;
use wind_sim::map::Map;
use wind_sim::mapfile::MapFile;
use wind_sim::mapgen::Layout;
use wind_sim::{Controller, SimConfig, Vec2, World};

fn config(search_radius: f32, per_team: u32) -> SimConfig {
    let mut c = SimConfig { soldiers_per_team: per_team, ..Default::default() };
    c.cover.search_radius = search_radius;
    c
}

/// «Руины»: поле 160 × 160 в обломках стен по 4 м, точка захвата посередине.
fn ruins() -> MapFile {
    let mut map = Map::new(160, 160);
    let point = Vec2::new(80.0, 80.0);
    for gy in 1..14 {
        for gx in 2..13 {
            let (x0, y0) = (gx * 12 + (gy % 2) * 6, gy * 11);
            if Vec2::new(x0 as f32, y0 as f32).distance(point) < 6.0 {
                continue;
            }
            for k in 0..4 {
                if (gx + gy) % 2 == 0 { map.set(x0 + k, y0, 120) } else { map.set(x0, y0 + k, 120) }
            }
        }
    }
    let layout = Layout { bases: [Vec2::new(8.0, 80.0), Vec2::new(152.0, 80.0)], capture_points: vec![point] };
    MapFile { name: "руины".into(), map, layout, atmosphere: Atmosphere::default() }
}

#[derive(Default)]
struct Sample {
    /// Сколько солдат ИИ в бою: видят врага или по ним стреляют.
    fighting: u32,
    /// Из них стоят у стены (ближе 1 м).
    at_wall: u32,
    /// В укрытии.
    in_cover: u32,
    /// Перезаряжаются в укрытии и уже ближе к месту «спрятаться», чем «стрелять».
    hiding: u32,
    reloading_in_cover: u32,
}

fn run(mut w: World, ticks: u32) -> Sample {
    let mut s = Sample::default();
    for tick in 0..ticks {
        w.step();
        if tick % 10 != 0 || tick < 400 {
            continue;
        }
        for x in w.soldiers.iter().filter(|x| x.alive && x.controller == Controller::Ai) {
            // За углом врага не видно — поэтому «в бою» и те, по кому стреляют.
            if x.target.is_some() || x.under_fire > 0.0 {
                s.fighting += 1;
                s.at_wall += w.map.circle_hits_solid(x.pos, 1.0) as u32;
                s.in_cover += x.cover.is_some() as u32;
            }
            if let Some(c) = x.cover
                && x.reload_timer > 0.5
            {
                s.reloading_in_cover += 1;
                s.hiding += (x.pos.distance(c.hide) < x.pos.distance(c.shoot)) as u32;
            }
        }
    }
    s
}

fn share(a: u32, b: u32) -> f32 {
    a as f32 / b.max(1) as f32
}

#[test]
fn soldiers_fight_from_corners_in_ruins() {
    let on = run(World::from_file(config(12.0, 40), 5, ruins()), 1600);
    let off = run(World::from_file(config(0.0, 40), 5, ruins()), 1600);
    println!(
        "руины, в бою у стены: с укрытиями {:.0}% ({:.0}% в укрытии), без — {:.0}%; на перезарядке прячутся {} из {}",
        share(on.at_wall, on.fighting) * 100.0,
        share(on.in_cover, on.fighting) * 100.0,
        share(off.at_wall, off.fighting) * 100.0,
        on.hiding,
        on.reloading_in_cover
    );
    assert!(on.fighting > 200 && off.fighting > 200, "бой должен идти: {} / {}", on.fighting, off.fighting);
    assert!(share(on.in_cover, on.fighting) > 0.3, "в руинах заметная доля дерущихся — в укрытии");
    assert!(share(on.at_wall, on.fighting) > share(off.at_wall, off.fighting) * 1.5, "с укрытиями у стен дерутся чаще");
    assert!(share(on.hiding, on.reloading_in_cover) > 0.6, "на перезарядке прячутся за угол");
    assert_eq!(off.in_cover, 0, "без поиска укрытий их нет");
}

#[test]
fn open_battle_still_fights() {
    // Процедурная карта: у точек чистое поле, укрытий мало — но бой идёт как раньше.
    let c = SimConfig { map_size: 256, capture_points: 3, ..config(12.0, 80) };
    let w = World::new(c, 5);
    let s = run(w, 1600);
    println!("поле, в бою {} (в укрытии {:.0}%)", s.fighting, share(s.in_cover, s.fighting) * 100.0);
    assert!(s.fighting > 300 && s.in_cover > 0);
}
