//! Лодки: стоят у баз, водитель правит, пассажиры стреляют, выходят на берег, от взрыва
//! тонут.

use wind_sim::boat::{NO_BOAT, hull_fits};
use wind_sim::map::SWIM_DEPTH;
use wind_sim::{Event, PlayerInput, Projectile, ProjectileKind, SimConfig, Vec2, World};

fn world() -> World {
    let mut c = SimConfig { map_size: 256, soldiers_per_team: 6, capture_points: 3, ..Default::default() };
    c.round.hold_time = f32::INFINITY;
    c.round.time_limit = 0.0;
    World::new(c, 5)
}

/// Игрок стоит рядом с лодкой `b` и жмёт V.
fn board(w: &mut World, me: u32, b: usize) {
    w.soldiers[me as usize].pos = w.boats[b].pos + Vec2::new(0.0, 2.0);
    let aim = w.soldiers[me as usize].pos + Vec2::X;
    w.set_input(me, PlayerInput { aim, use_boat: true, ..Default::default() });
    w.step();
    w.set_input(me, PlayerInput { aim, ..Default::default() });
    for _ in 0..10 {
        w.step();
    }
}

#[test]
fn boats_wait_on_the_water_near_bases() {
    let w = world();
    assert_eq!(w.boats.len(), 2, "по лодке на команду");
    for b in &w.boats {
        assert!(hull_fits(&w.map, b.pos, b.heading), "лодка на мели: {:?}", b.pos);
        assert!(b.seats.iter().all(|&s| s == NO_BOAT));
    }
}

#[test]
fn driver_steers_on_water_but_not_onto_land() {
    let mut w = world();
    let me = w.take_control(0).unwrap();
    board(&mut w, me, 0);
    assert_eq!(w.boats[0].seats[0], me, "первый севший — водитель");
    assert_eq!(w.soldiers[me as usize].boat, 0);
    // Плывём 6 с в одну сторону — до берега и упираемся.
    let start = w.boats[0].pos;
    let dir = Vec2::from_angle(w.boats[0].heading);
    let mut max_speed: f32 = 0.0;
    for _ in 0..120 {
        let p = w.soldiers[me as usize].pos;
        w.set_input(me, PlayerInput { move_dir: dir, aim: p + dir, ..Default::default() });
        w.step();
        max_speed = max_speed.max(w.boats[0].speed);
        assert!(hull_fits(&w.map, w.boats[0].pos, w.boats[0].heading), "лодка вылезла на сушу");
        // Водитель сидит в лодке.
        assert!(w.soldiers[me as usize].pos.distance(w.boats[0].pos) < 2.0);
    }
    println!("проплыла {:.1} м, скорость до {max_speed:.1} м/с", w.boats[0].pos.distance(start));
    assert!(w.boats[0].pos.distance(start) > 5.0, "лодка не поплыла");
    assert!(max_speed > 5.0);
    // Выходим — на сушу или мелководье рядом.
    let p = w.soldiers[me as usize].pos;
    w.set_input(me, PlayerInput { aim: p, use_boat: true, ..Default::default() });
    w.step();
    let s = &w.soldiers[me as usize];
    assert_eq!(s.boat, NO_BOAT);
    assert!(w.map.water_depth(s.pos) < SWIM_DEPTH, "вышел в глубину: {:.2} м", w.map.water_depth(s.pos));
    assert!(w.boats[0].seats.iter().all(|&o| o != me));
}

#[test]
fn passenger_shoots_driver_does_not_and_a_grenade_sinks_the_boat() {
    let mut w = world();
    let driver = w.take_control(0).unwrap();
    let passenger = w.take_control(0).unwrap();
    board(&mut w, driver, 0);
    board(&mut w, passenger, 0);
    assert_eq!(w.boats[0].seats[0], driver);
    assert!(w.boats[0].seats[1..].contains(&passenger));
    let shoots = |w: &mut World, who: u32| {
        let s = &mut w.soldiers[who as usize];
        s.cooldown = 0.0;
        s.reload_timer = 0.0;
        let aim = s.pos + Vec2::new(0.0, 15.0);
        w.set_input(who, PlayerInput { aim, fire: true, ..Default::default() });
        w.step();
        let shot = w.events.iter().any(|e| matches!(e, Event::Shot { shooter, .. } if *shooter == who));
        let aim = w.soldiers[who as usize].pos;
        w.set_input(who, PlayerInput { aim, ..Default::default() });
        shot
    };
    assert!(shoots(&mut w, passenger), "пассажир стреляет — он не плывёт, он в лодке");
    assert!(!shoots(&mut w, driver), "водитель правит, не стреляет");

    // Граната у борта: лодка тонет, сидящие — в воде или на берегу, не в лодке.
    let at = w.boats[0].pos + Vec2::new(0.5, 0.0);
    w.projectiles.push(Projectile { pos: at, vel: Vec2::ZERO, ttl: 0.01, team: 1, shooter: 7, weapon: 0, kind: ProjectileKind::Grenade { fuse: 0.01, flight: 0.0 } });
    for _ in 0..3 {
        w.step();
    }
    assert!(!w.boats[0].afloat(), "лодка не утонула");
    for who in [driver, passenger] {
        assert_eq!(w.soldiers[who as usize].boat, NO_BOAT);
    }
    // Через 30 с — снова у базы.
    let home = w.boats[0].home.0;
    for _ in 0..(31 * 20) {
        w.step();
    }
    assert!(w.boats[0].afloat() && w.boats[0].pos == home);
}
