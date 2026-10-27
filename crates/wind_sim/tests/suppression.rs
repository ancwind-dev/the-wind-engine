//! Подавление огнём, «залечь», огонь на подавление, отход и заживление раненых.

use wind_sim::cover::Cover;
use wind_sim::{Controller, Event, PlayerInput, Projectile, ProjectileKind, SimConfig, Vec2, World};

/// Двое на пустой карте: синий №0 и красный №1, далеко друг от друга.
fn duo() -> World {
    let mut w = World::new(SimConfig { map_size: 256, soldiers_per_team: 1, capture_points: 3, ..Default::default() }, 2);
    w.map = wind_sim::map::Map::new(256, 256);
    w.soldiers[0].pos = Vec2::new(30.0, 30.0);
    w.soldiers[1].pos = Vec2::new(220.0, 220.0);
    w
}

/// Солдат стоит на месте: им «управляет игрок», который ничего не жмёт.
fn hold_still(w: &mut World, i: usize) {
    w.soldiers[i].controller = Controller::Player(PlayerInput { aim: w.soldiers[i].pos + Vec2::X, ..Default::default() });
}

#[test]
fn pinned_in_the_open_goes_prone_in_cover_hides() {
    let mut w = duo();
    w.soldiers[1].suppression = 0.9;
    w.step();
    assert!(w.soldiers[1].pinned && w.soldiers[1].prone, "в чистом поле прижатый залегает");
    // Залёгший почти не двигается.
    let before = w.soldiers[1].pos;
    for _ in 0..10 {
        w.soldiers[1].suppression = 0.9;
        w.step();
    }
    assert!(w.soldiers[1].pos.distance(before) < 2.0);
    // Отпустило — встаёт.
    for _ in 0..80 {
        w.step();
    }
    assert!(!w.soldiers[1].pinned && !w.soldiers[1].prone);

    // В укрытии прижатый не ложится, а уходит за угол.
    let p = w.soldiers[1].pos;
    let cover = Cover { shoot: p, hide: p + Vec2::new(0.0, 2.0), threat: p + Vec2::new(20.0, 0.0) };
    w.soldiers[1].cover = Some(cover);
    w.soldiers[1].cover_timer = 10.0;
    for _ in 0..30 {
        w.soldiers[1].suppression = 0.9;
        w.step();
    }
    let s = &w.soldiers[1];
    assert!(s.pinned && !s.prone);
    assert!(s.pos.distance(cover.hide) < 0.5, "прячется за угол: {:?}", s.pos);
}

#[test]
fn prone_soldier_is_harder_to_hit() {
    // Пуля проходит в 0,27 м от центра: стоящего (радиус 0,35) задевает, лежачего — нет.
    let hit = |prone: bool| {
        let mut w = duo();
        let target = Vec2::new(120.0, 120.0);
        w.soldiers[1].pos = target;
        if prone {
            w.soldiers[1].suppression = 1.0;
        } else {
            hold_still(&mut w, 1);
        }
        w.step();
        assert_eq!(w.soldiers[1].prone, prone);
        let start = Vec2::new(target.x - 6.0, w.soldiers[1].pos.y + 0.27);
        w.projectiles.push(Projectile { pos: start, vel: Vec2::new(90.0, 0.0), ttl: 1.0, team: 0, shooter: 0, weapon: 0, kind: ProjectileKind::Bullet });
        w.step();
        w.step();
        w.soldiers[1].health < 1.0 || !w.soldiers[1].alive
    };
    assert!(hit(false), "в стоящего попала");
    assert!(!hit(true), "лежачего не задела");
}

#[test]
fn near_misses_suppress() {
    let mut w = duo();
    hold_still(&mut w, 1);
    let at = w.soldiers[1].pos;
    for k in 0..6 {
        // Очередь мимо — в полутора метрах.
        let start = Vec2::new(at.x - 2.0, at.y + 1.5);
        w.projectiles.push(Projectile { pos: start, vel: Vec2::new(90.0, 0.0), ttl: 1.0, team: 0, shooter: 0, weapon: 0, kind: ProjectileKind::Bullet });
        w.step();
        assert!(w.soldiers[1].suppression > 0.0, "после пули {k} — подавлен");
    }
    assert!(w.soldiers[1].under_fire > 0.0 && w.soldiers[1].threat.is_some());
    assert!(w.soldiers[1].suppression > 0.5, "очередь рядом прижимает: {}", w.soldiers[1].suppression);
}

#[test]
fn rifleman_keeps_firing_where_the_enemy_hid() {
    let mut w = duo();
    // Синий видел врага в 20 м, тот исчез; синий стоит и бьёт туда.
    let spot = w.soldiers[0].pos + Vec2::new(20.0, 0.0);
    w.soldiers[0].last_seen = Some(spot);
    w.soldiers[0].last_seen_timer = 2.0;
    // При появлении у каждого случайная пауза до первого выстрела — убираем её.
    w.soldiers[0].cooldown = 0.0;
    let mut shots = 0;
    for _ in 0..20 {
        w.step();
        shots += w.events.iter().filter(|e| matches!(e, Event::Shot { shooter: 0, .. })).count();
    }
    assert!(shots >= 3, "огонь на подавление: {shots} выстрелов");
    // Через 2 с перестаёт.
    for _ in 0..40 {
        w.step();
    }
    w.events.clear();
    w.step();
    assert!(!w.events.iter().any(|e| matches!(e, Event::Shot { shooter: 0, .. })));
}

#[test]
fn wounded_retreat_and_heal() {
    let mut w = duo();
    let base = w.bases[1];
    // Красный ранен в поле, по нему стреляли с запада.
    w.soldiers[1].pos = base + Vec2::new(-60.0, 0.0);
    w.soldiers[1].health = 0.4;
    w.soldiers[1].threat = Some(base + Vec2::new(-100.0, 0.0));
    w.soldiers[1].under_fire = 1.0;
    let start = w.soldiers[1].pos.distance(base);
    for _ in 0..200 {
        w.step();
    }
    let s = &w.soldiers[1];
    assert!(s.retreat.is_some(), "раненый отходит");
    assert!(s.pos.distance(base) < start - 30.0, "отошёл к базе: {} → {}", start, s.pos.distance(base));
    // Вне боя заживает и возвращается в строй.
    for _ in 0..800 {
        w.step();
    }
    let s = &w.soldiers[1];
    assert!(s.health >= 0.9, "зажило: {}", s.health);
    assert!(s.retreat.is_none());
}

/// Бой 2 минуты, 80 на 80: (убийств, доля залёгших среди дерущихся, сколько раз
/// в замерах видели отходящих раненых).
fn battle(on: bool) -> (u32, f32, u32) {
    let mut c = SimConfig { map_size: 256, soldiers_per_team: 80, capture_points: 3, ..Default::default() };
    if !on {
        c.suppression.prone_at = 2.0;
        c.suppression.spread_penalty = 0.0;
        c.suppression.suppress_memory = 0.0;
        c.suppression.retreat_below = 0.0;
    }
    let mut w = World::new(c, 5);
    let (mut fighting, mut prone, mut retreating) = (0u32, 0u32, 0u32);
    for tick in 0..2400 {
        w.step();
        if tick % 10 == 0 && tick > 400 {
            for s in w.soldiers.iter().filter(|s| s.alive && s.controller == Controller::Ai) {
                if s.target.is_some() || s.under_fire > 0.0 {
                    fighting += 1;
                    prone += s.prone as u32;
                }
                retreating += s.retreat.is_some() as u32;
            }
        }
    }
    (w.score[0] + w.score[1], prone as f32 / fighting.max(1) as f32, retreating)
}

#[test]
fn battle_with_suppression() {
    let (kills_on, prone, retreats) = battle(true);
    let (kills_off, prone_off, retreats_off) = battle(false);
    println!(
        "убийств за 2 мин: с подавлением {kills_on}, без — {kills_off}; залегли {:.0}% дерущихся; отходящих раненых в замерах {retreats}",
        prone * 100.0
    );
    assert!(prone > 0.05, "под огнём залегают");
    assert!(retreats > 20, "раненые отходят");
    assert_eq!((prone_off, retreats_off), (0.0, 0));
    assert!(kills_on as f32 > kills_off as f32 * 0.4, "бой идёт, а не все лежат");
}
