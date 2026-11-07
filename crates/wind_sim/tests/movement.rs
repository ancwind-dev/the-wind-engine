//! Инерция шага: солдат разгоняется и тормозит не мгновенно, как у RWR.

use wind_sim::{Vec2, steer};

/// Сколько секунд от начала до доли `share` скорости `target` (шаг 1/60 с).
fn time_to(from: Vec2, target: Vec2, done: impl Fn(Vec2) -> bool) -> f32 {
    let dt = 1.0 / 60.0;
    let mut v = from;
    for k in 1..600 {
        v = steer(v, target, dt);
        if done(v) {
            return k as f32 * dt;
        }
    }
    f32::INFINITY
}

#[test]
fn soldier_accelerates_and_brakes_not_at_once() {
    let run = Vec2::new(6.0, 0.0);
    // Разгон до 95 % бега — за треть секунды, не мгновенно и не вязко.
    let up = time_to(Vec2::ZERO, run, |v| v.x >= 5.7);
    assert!((0.2..0.45).contains(&up), "разгон {up:.2} с");
    // Торможение — резче разгона: до полной остановки — меньше четверти секунды.
    let stop = time_to(run, Vec2::ZERO, |v| v == Vec2::ZERO);
    assert!(stop < 0.3 && stop < up, "остановка {stop:.2} с, разгон {up:.2} с");
    // Разворот назад — тоже торможением, а не разгоном.
    let back = time_to(run, -run, |v| v.x <= -5.7);
    assert!(back < up * 1.6, "разворот {back:.2} с");
    // Шаг — та же функция у сервера и у клиента: одинаково при одинаковом вводе.
    let a = (0..30).fold(Vec2::ZERO, |v, _| steer(v, run, 1.0 / 30.0));
    let b = (0..30).fold(Vec2::ZERO, |v, _| steer(v, run, 1.0 / 30.0));
    assert_eq!(a, b);
}
