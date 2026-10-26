//! Оружие и классы в бою — на настоящих данных из `data/`.

use std::collections::HashMap;
use std::path::Path;
use wind_sim::{Event, PlayerInput, SimConfig, Vec2, World};

fn config(per_team: u32) -> SimConfig {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/sim.ron");
    let mut c = SimConfig::load(&path).unwrap();
    c.map_size = 256;
    c.soldiers_per_team = per_team;
    c.capture_points = 3;
    c
}

#[test]
fn classes_fight_with_their_own_weapons() {
    let mut world = World::new(config(80), 5);
    let mut shots: HashMap<String, u32> = HashMap::new();
    let mut shells = 0;
    let mut explosions = 0;
    let mut shell_shooters = Vec::new();
    let mut kills_by_launchers = 0;
    for _ in 0..1800 {
        world.step();
        for e in &world.events {
            match *e {
                Event::Shot { shooter, .. } => *shots.entry(world.weapon(shooter).name.clone()).or_default() += 1,
                Event::GrenadeThrown { shooter, .. } if world.weapon(shooter).blast.is_some() => {
                    shells += 1;
                    shell_shooters.push(shooter);
                }
                Event::Explosion { .. } => explosions += 1,
                Event::Kill { killer, .. } if world.weapon(killer).blast.is_some() => kills_by_launchers += 1,
                _ => {}
            }
        }
    }
    println!("выстрелов по оружию: {shots:?}; снарядов гранатомётов: {shells}, взрывов: {explosions}, убито гранатомётчиками: {kills_by_launchers}");
    // Каждое стрелковое оружие из data/factions.ron стреляло.
    for name in ["m4", "pp2000", "ak74", "aks74u"] {
        assert!(shots.get(name).is_some_and(|&n| n > 0), "из {name} никто не стрелял");
    }
    // У гранатомётов — снаряды, а не пули, и они взрываются.
    assert!(!shots.contains_key("m32_mgl") && !shots.contains_key("gm94"), "гранатомёт стрелял пулями");
    assert!(shells > 0, "гранатомётчики не стреляли");
    assert!(explosions >= shells, "не все снаряды взорвались: {explosions} < {shells}");
    assert!(kills_by_launchers > 0, "гранатомётчики никого не убили");
}

#[test]
fn long_burst_spreads_wider() {
    let mut world = World::new(config(20), 9);
    // Берём солдата с M4 и стреляем на восток, пока враги далеко.
    let me = (0..world.soldiers.len() as u32).find(|&i| world.soldiers[i as usize].team == 0 && world.weapon(i).name == "m4").unwrap();
    world.take_control(0);
    let s = &mut world.soldiers[me as usize];
    s.controller = wind_sim::Controller::Player(PlayerInput::default());
    s.cooldown = 0.0;
    let mut angles = Vec::new();
    // Три секунды без отпускания: 30 патронов M4 как раз уходят.
    for _ in 0..60 {
        let pos = world.soldiers[me as usize].pos;
        world.set_input(me, PlayerInput { aim: pos + Vec2::new(30.0, 0.0), fire: true, ..Default::default() });
        world.step();
        for e in &world.events {
            if let Event::Shot { shooter, angle } = *e
                && shooter == me
            {
                angles.push(angle.abs());
            }
        }
    }
    // M4: 8,8 выстрела в секунду — за 3 секунды весь магазин.
    assert!(angles.len() >= 24, "мало выстрелов: {}", angles.len());
    let first = angles[..4].iter().copied().fold(0.0, f32::max);
    let late = angles[angles.len() - 8..].iter().copied().fold(0.0, f32::max);
    println!("отклонение: первые выстрелы до {:.1}°, конец очереди до {:.1}°", first.to_degrees(), late.to_degrees());
    assert!(late > first, "в длинной очереди разброс должен расти");
}
