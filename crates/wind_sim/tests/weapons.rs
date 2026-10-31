//! Оружие и классы в бою — на настоящих данных из `data/`.

use std::collections::HashMap;
use std::path::Path;
use wind_sim::{Event, PlayerInput, ProjectileKind, SimConfig, Vec2, World};

fn config(per_team: u32) -> SimConfig {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/sim.ron");
    let mut c = SimConfig::load(&path).unwrap();
    c.map_size = 256;
    c.soldiers_per_team = per_team;
    c.capture_points = 3;
    // Раунд не кончается: при конце раунда снаряды в полёте убираются, а тест — про оружие.
    c.round.hold_time = f32::INFINITY;
    c.round.time_limit = 0.0;
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
    // 3 минуты: со штабом силы расходятся по точкам, и у PP-2000 (короткая дальность)
    // стычки начинаются не сразу.
    for _ in 0..3600 {
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
    // Снаряд, выпущенный в последние секунды, может быть ещё в полёте.
    let flying = world.projectiles.iter().filter(|p| matches!(p.kind, ProjectileKind::Shell { .. })).count();
    assert!(world.round.playing());
    assert!(explosions + flying >= shells, "не все снаряды взорвались: {explosions} + {flying} в полёте < {shells}");
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

#[test]
fn player_reloads_on_demand() {
    let mut world = World::new(config(20), 9);
    let me = (0..world.soldiers.len() as u32).find(|&i| world.soldiers[i as usize].team == 0 && world.weapon(i).name == "m4").unwrap();
    world.take_control(0);
    let s = &mut world.soldiers[me as usize];
    s.controller = wind_sim::Controller::Player(PlayerInput::default());
    s.cooldown = 0.0;
    let full = world.weapon(me).magazine_size;
    let aim = |w: &World| w.soldiers[me as usize].pos + Vec2::new(30.0, 0.0);

    // С полным магазином R ничего не делает.
    world.set_input(me, PlayerInput { aim: aim(&world), reload: true, ..Default::default() });
    world.step();
    assert_eq!(world.soldiers[me as usize].reload_timer, 0.0, "полный магазин не перезаряжают");

    // Пара выстрелов — и R, не дожидаясь пустого магазина.
    for _ in 0..4 {
        world.set_input(me, PlayerInput { aim: aim(&world), fire: true, ..Default::default() });
        world.step();
    }
    let left = world.soldiers[me as usize].ammo;
    assert!(left > 0 && left < full, "патронов: {left} из {full}");
    world.set_input(me, PlayerInput { aim: aim(&world), reload: true, fire: true, ..Default::default() });
    world.step();
    assert!(world.soldiers[me as usize].reload_timer > 0.0, "перезарядка не началась");

    // Во время перезарядки не стреляет, после — магазин полный.
    let mut shots = 0;
    for _ in 0..200 {
        world.set_input(me, PlayerInput { aim: aim(&world), fire: world.soldiers[me as usize].reload_timer > 0.0, ..Default::default() });
        world.step();
        shots += world.events.iter().filter(|e| matches!(e, Event::Shot { shooter, .. } if *shooter == me)).count();
        if world.soldiers[me as usize].reload_timer <= 0.0 {
            break;
        }
    }
    assert_eq!(shots, 0, "стрелял во время перезарядки");
    assert_eq!(world.soldiers[me as usize].ammo, full);
}

#[test]
fn thrown_grenade_is_shown_briefly() {
    let mut world = World::new(config(20), 9);
    let me = world.take_control(0).unwrap();
    let pos = world.soldiers[me as usize].pos;
    world.soldiers[me as usize].grenade_timer = 0.0;
    world.set_input(me, PlayerInput { aim: pos + Vec2::new(15.0, 0.0), grenade: true, ..Default::default() });
    world.step();
    let s = &world.soldiers[me as usize];
    assert!(s.throw_timer > 0.0, "бросок не отмечен");
    // Через полсекунды — уже не «бросает».
    world.set_input(me, PlayerInput { aim: pos, ..Default::default() });
    for _ in 0..10 {
        world.step();
    }
    assert_eq!(world.soldiers[me as usize].throw_timer, 0.0);
}

#[test]
fn player_runs_by_default_and_walks_with_shift() {
    // Сколько метров за секунду пройдёт игрок, идущий на восток.
    let distance = |walk: bool| {
        let mut world = World::new(config(20), 9);
        let me = world.take_control(0).unwrap();
        let start = world.soldiers[me as usize].pos;
        for _ in 0..20 {
            let pos = world.soldiers[me as usize].pos;
            world.set_input(me, PlayerInput { move_dir: Vec2::X, aim: pos + Vec2::X, walk, ..Default::default() });
            world.step();
        }
        world.soldiers[me as usize].pos.distance(start)
    };
    let (run, walk) = (distance(false), distance(true));
    println!("за секунду: бегом {run:.2} м, шагом {walk:.2} м");
    assert!(run > 4.0, "бегом — скорость бега ({run:.2} м/с)");
    assert!((walk / run - wind_sim::world::WALK_SHARE).abs() < 0.08, "шагом — {:.0} % бега", walk / run * 100.0);
}
