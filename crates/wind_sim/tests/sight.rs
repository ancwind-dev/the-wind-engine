//! Обзор и стрельба ИИ, как в RWR: кого и с какого расстояния замечает, задержка
//! перед первым выстрелом, очереди с паузами.

use wind_sim::{Controller, Event, PlayerInput, SimConfig, Vec2, World};

/// Синий ИИ №0 и красный №1 (им «управляет игрок») на пустой карте в `d` м друг от друга.
/// Красный идёт со скоростью `speed` поперёк линии взгляда, или стоит, или стреляет в сторону.
fn pair(d: f32) -> World {
    let mut c = SimConfig { map_size: 256, soldiers_per_team: 1, capture_points: 3, ..Default::default() };
    c.round.hold_time = f32::INFINITY;
    c.round.time_limit = 0.0;
    let mut w = World::new(c, 3);
    w.map = wind_sim::map::Map::new(256, 256);
    w.points.iter_mut().for_each(|p| p.pos = Vec2::new(10.0, 240.0));
    w.soldiers[0].pos = Vec2::new(60.0, 128.0);
    w.soldiers[1].pos = Vec2::new(60.0 + d, 128.0);
    w
}

/// Заметит ли синий красного за 2 с: красный бежит, стоит или стреляет в сторону.
fn spots(d: f32, red: &str) -> bool {
    let mut w = pair(d);
    for _ in 0..40 {
        let p = w.soldiers[1].pos;
        let input = match red {
            "бежит" => PlayerInput { move_dir: Vec2::Y * if w.tick % 40 < 20 { 1.0 } else { -1.0 }, aim: p - Vec2::Y * 30.0, ..Default::default() },
            "стреляет" => PlayerInput { aim: p + Vec2::new(10.0, 30.0), fire: true, ..Default::default() },
            _ => PlayerInput { aim: p - Vec2::Y * 30.0, ..Default::default() },
        };
        w.soldiers[1].controller = Controller::Player(input);
        // Синий стоит, где стоял: проверяем только глаза.
        w.soldiers[0].pos = Vec2::new(60.0, 128.0);
        w.step();
        if w.soldiers[0].target == Some(1) {
            return true;
        }
    }
    false
}

#[test]
fn running_is_seen_farther_than_standing_and_shooting_farther_still() {
    let vision = SimConfig::default().soldier.vision_range;
    assert_eq!(vision, 32.0, "обзор ИИ — как в RWR");
    // Стоящего (0,88 обзора — 28 м) в 30 м не видно, бегущего — видно.
    assert!(!spots(30.0, "стоит"));
    assert!(spots(30.0, "бежит"));
    // Дальше обзора — не видно бегущего, но видно стреляющего (×1,7, до 40 м).
    assert!(!spots(36.0, "бежит"));
    assert!(spots(36.0, "стреляет"));
    assert!(!spots(45.0, "стреляет"));
    // Близко — видно и стоящего.
    assert!(spots(20.0, "стоит"));
}

#[test]
fn ai_aims_before_the_first_shot_and_fires_in_bursts() {
    let mut w = pair(15.0);
    w.soldiers[1].controller = Controller::Player(PlayerInput { aim: w.soldiers[1].pos - Vec2::Y * 30.0, ..Default::default() });
    w.soldiers[0].cooldown = 0.0;
    let mut seen_at = None;
    let mut shots = Vec::new();
    for t in 0..200u32 {
        w.step();
        // Красного не убить: меряем только, когда и как синий стреляет.
        let red = &mut w.soldiers[1];
        red.health = 1.0;
        red.alive = true;
        red.pos = Vec2::new(75.0, 128.0);
        if seen_at.is_none() && w.soldiers[0].target == Some(1) {
            seen_at = Some(t);
        }
        if w.events.iter().any(|e| matches!(e, Event::Shot { shooter: 0, .. })) {
            shots.push(t);
        }
    }
    let seen = seen_at.expect("в 15 м стоящего видно");
    let first = *shots.first().expect("стрелял");
    let delay = (first - seen) as f32 * 0.05;
    println!("заметил на тике {seen}, первый выстрел через {delay:.2} с; выстрелов {}", shots.len());
    assert!((0.25..=0.75).contains(&delay), "реакция 0,3–0,6 с, а не сразу: {delay:.2} с");
    // Очереди: между выстрелами бывают паузы больше полсекунды, и их несколько.
    let pauses = shots.windows(2).filter(|p| p[1] - p[0] > 10).count();
    assert!(pauses >= 3, "стреляет очередями с паузами: пауз {pauses}");
    // И не сплошь: за 10 с M4 без пауз выпустил бы около 80 пуль (с перезарядками).
    assert!(shots.len() < 50, "слишком часто: {}", shots.len());
}
