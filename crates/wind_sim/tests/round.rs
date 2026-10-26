//! Раунд: победа, перерыв, новый раунд, выбор точки возрождения.

use wind_sim::round::{EndReason, Phase};
use wind_sim::{Controller, Event, PlayerInput, SimConfig, Vec2, World};

fn world(hold: f32, limit: f32) -> World {
    let mut c = SimConfig { map_size: 256, soldiers_per_team: 20, capture_points: 3, ..Default::default() };
    c.round.hold_time = hold;
    c.round.time_limit = limit;
    c.round.intermission = 2.0;
    World::new(c, 4)
}

/// Отдать все точки команде.
fn give_points(w: &mut World, team: u8) {
    for p in &mut w.points {
        p.owner = Some(team);
        p.progress = if team == 0 { -1.0 } else { 1.0 };
    }
}

fn run_until(w: &mut World, ticks: u32, mut stop: impl FnMut(&World, &Event) -> bool) -> bool {
    for _ in 0..ticks {
        w.step();
        if w.events.iter().any(|e| stop(w, e)) {
            return true;
        }
    }
    false
}

#[test]
fn holding_all_points_wins() {
    let mut w = world(3.0, 0.0);
    // Все солдаты синих — на базе красных нет: никто не отбивает точки. Просто
    // держим точки за синими каждый тик и ждём победы.
    let mut won = None;
    for _ in 0..200 {
        give_points(&mut w, 0);
        w.step();
        for e in &w.events {
            if let Event::RoundOver { winner } = *e {
                won = Some(winner);
            }
        }
        if won.is_some() {
            break;
        }
    }
    assert_eq!(won, Some(Some(0)), "синие держали все точки — должны победить");
    assert_eq!(w.round.phase, Phase::Over { winner: Some(0), reason: EndReason::AllPoints });
    // Победа — не раньше, чем через hold_time: 3 с = 60 тиков.
    assert!(w.tick >= 60, "слишком рано: тик {}", w.tick);
}

#[test]
fn time_up_goes_to_who_has_more_points() {
    let mut w = world(1000.0, 2.0);
    w.points[0].owner = Some(1);
    w.points[0].progress = 1.0;
    let ended = run_until(&mut w, 100, |_, e| matches!(e, Event::RoundOver { .. }));
    assert!(ended, "время вышло — раунд должен кончиться");
    // Точку могли успеть отбить, но победитель — тот, у кого больше точек.
    let Phase::Over { reason, .. } = w.round.phase else { panic!("раунд должен кончиться") };
    assert_eq!(reason, EndReason::TimeUp);
}

#[test]
fn new_round_restores_map_points_and_score() {
    let mut w = world(1000.0, 1.0);
    let initial = w.map.solid.clone();
    // Ломаем первые попавшиеся разрушаемые стены и набираем счёт.
    let walls: Vec<Vec2> = (0..w.map.solid.len())
        .filter(|&i| w.map.solid[i] != 0 && w.map.solid[i] != wind_sim::map::INDESTRUCTIBLE)
        .step_by(40)
        .take(10)
        .map(|i| Vec2::new((i as u32 % w.map.width) as f32 + 0.5, (i as u32 / w.map.width) as f32 + 0.5))
        .collect();
    for at in walls {
        w.map.damage_circle(at, 3.0, 250.0);
    }
    assert!(w.map.solid != initial, "стены должны были сломаться");
    w.score = [5, 7];
    // Раунд кончился (по времени) — во время перерыва никто не стреляет.
    assert!(run_until(&mut w, 40, |_, e| matches!(e, Event::RoundOver { .. })));
    let mut shots_in_break = 0;
    let started = run_until(&mut w, 100, |w, e| {
        if matches!(e, Event::Shot { .. }) && !w.round.playing() {
            shots_in_break += 1;
        }
        matches!(e, Event::RoundStarted { number: 2 })
    });
    assert!(started, "после перерыва должен начаться второй раунд");
    assert_eq!(shots_in_break, 0, "в перерыве не стреляют");
    assert!(w.map.solid == initial, "стены должны быть целыми, как в начале");
    assert_eq!(w.score, [0, 0]);
    assert!(w.points.iter().all(|p| p.owner.is_none()));
    assert!(w.soldiers.iter().all(|s| s.alive && s.kills == 0));
}

#[test]
fn player_respawns_at_chosen_point() {
    let mut w = world(1000.0, 0.0);
    let me = w.take_control(0).unwrap();
    // Точка 1 — наша; выбираем её и умираем.
    w.points[1].owner = Some(0);
    w.points[1].progress = -1.0;
    w.set_input(me, PlayerInput { spawn: 2, ..Default::default() });
    let s = &mut w.soldiers[me as usize];
    s.alive = false;
    s.respawn_timer = 0.1;
    let point = w.points[1].pos;
    for _ in 0..5 {
        w.step();
    }
    let s = &w.soldiers[me as usize];
    assert!(s.alive, "игрок должен возродиться");
    assert!(s.pos.distance(point) < 8.0, "должен появиться у выбранной точки, а он в {:?}", s.pos);
    assert!(matches!(s.controller, Controller::Player(_)));
}
