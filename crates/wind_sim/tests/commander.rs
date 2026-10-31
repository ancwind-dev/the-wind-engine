//! Игрок-командир: звания, набор бойцов, «за мной», «держать», «распустить».

use wind_sim::commander::{Command, NO_COMMANDER};
use wind_sim::{Controller, PlayerInput, SimConfig, Vec2, World};

fn world() -> (World, u32) {
    let mut c = SimConfig { map_size: 256, soldiers_per_team: 12, capture_points: 3, ..Default::default() };
    c.round.hold_time = f32::INFINITY;
    c.round.time_limit = 0.0;
    let mut w = World::new(c, 4);
    let me = w.take_control(0).unwrap();
    (w, me)
}

/// Свои бойцы под ИИ, кроме игрока, — по порядку номеров.
fn friends(w: &World, me: u32) -> Vec<u32> {
    (0..w.soldiers.len() as u32).filter(|&i| i != me && w.soldiers[i as usize].team == 0 && w.soldiers[i as usize].controller == Controller::Ai).collect()
}

/// Игрок стоит и отдаёт приказ, прицел — в `aim`.
fn order(w: &mut World, me: u32, aim: Vec2, command: Command, ticks: u32) {
    for _ in 0..ticks {
        w.set_input(me, PlayerInput { aim, command, ..Default::default() });
        w.step();
    }
}

#[test]
fn kills_and_captures_raise_rank_and_squad_size() {
    let (w, _) = world();
    let r = &w.config.ranks;
    assert_eq!(r.squad_size(0), 0, "рядовой бойцов не ведёт");
    let mut last = 0;
    for xp in 0..200 {
        let size = r.squad_size(xp);
        assert!(size >= last, "со званием отряд не уменьшается");
        last = size;
    }
    assert!(last >= 8);
    assert_eq!(r.ranks[r.rank_of(15)].name, "Сержант");
}

#[test]
fn recruit_one_at_a_time_up_to_the_rank_limit() {
    let (mut w, me) = world();
    let at = w.soldiers[me as usize].pos + Vec2::new(5.0, 0.0);
    let f = friends(&w, me);
    // Трое своих стоят у прицела.
    for (k, &i) in f[..3].iter().enumerate() {
        w.soldiers[i as usize].pos = at + Vec2::new(0.0, k as f32 * 1.5);
    }
    // Рядовой — никого не позвать.
    order(&mut w, me, at, Command::Recruit, 3);
    assert!(w.squad_of(me).is_empty());
    // Ефрейтор (один боец): приказ держится несколько тиков, но зовёт одного.
    w.soldiers[me as usize].xp = w.config.ranks.ranks[1].xp;
    for &i in &f[..3] {
        w.soldiers[i as usize].pos = at + Vec2::new(0.0, (i % 3) as f32 * 1.5);
    }
    order(&mut w, me, at, Command::Recruit, 5);
    assert_eq!(w.squad_of(me).len(), 1);
    // Сержант (трое): каждый новый приказ — ещё один.
    w.soldiers[me as usize].xp = w.config.ranks.ranks[3].xp;
    for &i in &f[..3] {
        if w.soldiers[i as usize].commander == NO_COMMANDER {
            let aim = w.soldiers[i as usize].pos;
            order(&mut w, me, aim, Command::Recruit, 2);
        }
    }
    assert_eq!(w.squad_of(me).len(), 3);
    // Распустить — все возвращаются к своим отрядам.
    order(&mut w, me, at, Command::Dismiss, 2);
    assert!(w.squad_of(me).is_empty());
    assert!(w.soldiers.iter().all(|s| s.commander == NO_COMMANDER));
}

/// Набрать `n` ближайших своих (звание — с запасом).
fn recruit(w: &mut World, me: u32, n: usize) -> Vec<u32> {
    w.soldiers[me as usize].xp = 1000;
    let pos = w.soldiers[me as usize].pos;
    for (k, &i) in friends(w, me)[..n].iter().enumerate() {
        w.soldiers[i as usize].pos = pos + Vec2::new(-2.0, k as f32 * 1.2 - 3.0);
        let aim = w.soldiers[i as usize].pos;
        order(w, me, aim, Command::Recruit, 2);
    }
    let squad = w.squad_of(me);
    assert_eq!(squad.len(), n);
    squad
}

#[test]
fn squad_follows_the_player() {
    let (mut w, me) = world();
    let squad = recruit(&mut w, me, 4);
    // Игрок идёт 10 с к центру карты.
    let goal = Vec2::splat(128.0);
    for _ in 0..200 {
        let pos = w.soldiers[me as usize].pos;
        let dir = (goal - pos).normalize_or_zero();
        w.set_input(me, PlayerInput { move_dir: dir, aim: pos + dir * 10.0, ..Default::default() });
        w.step();
    }
    let me_pos = w.soldiers[me as usize].pos;
    let far = squad.iter().map(|&m| w.soldiers[m as usize].pos.distance(me_pos)).fold(0.0, f32::max);
    println!("игрок прошёл до {me_pos:?}, дальний боец в {far:.1} м");
    assert!(w.squad_of(me).len() >= 3, "отряд не разбежался");
    assert!(far < 15.0, "бойцы отстали от игрока: {far:.1} м");
}

#[test]
fn squad_holds_the_point_it_was_sent_to() {
    let (mut w, me) = world();
    let squad = recruit(&mut w, me, 3);
    let pos = w.soldiers[me as usize].pos;
    let at = pos + (Vec2::splat(128.0) - pos).normalize() * 20.0;
    order(&mut w, me, at, Command::Hold, 3);
    // Игрок уходит в другую сторону, отряд остаётся у точки.
    for _ in 0..200 {
        let p = w.soldiers[me as usize].pos;
        w.set_input(me, PlayerInput { move_dir: (pos - at).normalize(), aim: p, ..Default::default() });
        w.step();
    }
    let far = squad.iter().filter(|&&m| w.soldiers[m as usize].alive).map(|&m| w.soldiers[m as usize].pos.distance(at)).fold(0.0, f32::max);
    println!("дальний боец от удерживаемой точки — {far:.1} м");
    assert!(far < 10.0, "отряд не держит точку: {far:.1} м");
    // «За мной» — снова к игроку.
    order(&mut w, me, at, Command::Follow, 200);
    let me_pos = w.soldiers[me as usize].pos;
    let far = squad.iter().filter(|&&m| w.soldiers[m as usize].alive).map(|&m| w.soldiers[m as usize].pos.distance(me_pos)).fold(0.0, f32::max);
    assert!(far < 12.0, "по «за мной» отряд не вернулся: {far:.1} м");
}
