//! Отряды: идут вместе, строем, и после смерти возвращаются к своим.

use wind_sim::squad::Order;
use wind_sim::{SimConfig, Vec2, World};

fn world(squad_size: u32) -> World {
    let mut c = SimConfig { map_size: 512, soldiers_per_team: 36, capture_points: 5, ..Default::default() };
    c.squad.size = squad_size;
    World::new(c, 11)
}

/// Доля живых солдат ИИ дальше 20 м от первого живого из той же шестёрки (по
/// номерам — так же, как делятся отряды). Без отрядов — те же шестёрки, для сравнения.
fn stragglers(w: &World) -> f32 {
    let (mut far, mut n) = (0, 0);
    for team in 0..2u8 {
        let ids: Vec<usize> = (0..w.soldiers.len()).filter(|&i| w.soldiers[i].team == team).collect();
        for group in ids.chunks(6) {
            let alive: Vec<Vec2> = group.iter().map(|&i| &w.soldiers[i]).filter(|s| s.alive).map(|s| s.pos).collect();
            if let Some((&first, rest)) = alive.split_first() {
                for p in rest {
                    far += (p.distance(first) > 20.0) as u32;
                    n += 1;
                }
            }
        }
    }
    far as f32 / n.max(1) as f32
}

/// Насколько бойцы в среднем далеко от своих мест в строю, м.
fn slot_error(w: &World) -> f32 {
    let (mut sum, mut n) = (0.0, 0);
    for (i, o) in w.orders.iter().enumerate() {
        if let Order::Follow { slot, .. } = *o {
            sum += w.soldiers[i].pos.distance(slot);
            n += 1;
        }
    }
    sum / n.max(1) as f32
}

#[test]
fn squads_march_together() {
    let mut with = world(6);
    let mut without = world(0);
    let (mut a, mut b, mut err) = (0.0, 0.0, 0.0);
    // Первые 40 с: отряды расходятся к точкам, до большого боя.
    let samples = 30.0;
    for tick in 0..800 {
        with.step();
        without.step();
        if tick % 20 == 0 && tick >= 200 {
            a += stragglers(&with) / samples;
            b += stragglers(&without) / samples;
            err += slot_error(&with) / samples;
        }
    }
    println!("отбились дальше 20 м: с отрядами {:.0}%, без — {:.0}%; от места в строю в среднем {err:.1} м", a * 100.0, b * 100.0);
    assert!(a < b * 0.5, "отряд должен держаться вместе: {a} против {b}");
    assert!(err < 4.0, "бойцы — у своих мест в строю: {err} м");
}

#[test]
fn marching_squad_keeps_wedge_behind_leader() {
    let mut w = world(6);
    for _ in 0..200 {
        w.step();
    }
    // Командиры, которые идут: их бойцы — позади (клин), а не впереди.
    let (mut behind, mut total) = (0, 0);
    for q in &w.squads {
        let Some(l) = q.leader.map(|l| &w.soldiers[l as usize]) else { continue };
        if l.vel.length() < 2.0 {
            continue;
        }
        for &m in &q.members {
            if let Order::Follow { slot, heading, .. } = w.orders[m as usize] {
                total += 1;
                if (slot - l.pos).dot(heading) < 0.0 {
                    behind += 1;
                }
            }
        }
    }
    assert!(total > 10, "должны быть идущие отряды: {total}");
    assert!(behind as f32 >= total as f32 * 0.9, "места в клину — позади командира: {behind} из {total}");
}

#[test]
fn whole_squad_goes_to_one_point() {
    let mut w = world(6);
    for _ in 0..100 {
        w.step();
    }
    for q in w.squads.iter().filter(|q| q.leader.is_some()) {
        assert!(q.members.iter().all(|&m| w.soldiers[m as usize].objective == q.objective), "у отряда одна цель");
    }
}
