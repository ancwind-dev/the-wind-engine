//! ИИ солдат: кого видим, куда идём, бросать ли гранату.
//!
//! Солдат думает не каждый тик, а раз в `think_interval` тиков, и все — вразнобой.
//! Сами решения считаются параллельно: каждый поток только читает мир.
//! К какой точке идти, решает командир отряда — за весь отряд (см. `squad`).

use crate::rng::Rng;
use crate::world::{CapturePoint, Controller, NO_OBJECTIVE, World};
use glam::Vec2;
use rayon::prelude::*;

struct Decision {
    soldier: u32,
    target: Option<u32>,
    objective: u32,
    grenade_at: Option<Vec2>,
}

/// Больше кандидатов на видимость не проверяем: самые дальние всё равно не важны.
const MAX_CANDIDATES: usize = 8;

/// Возвращает, сколько солдат подумало.
pub(crate) fn think(world: &mut World) -> u32 {
    let interval = world.config.think_interval.max(1) as u64;
    let phase = world.tick % interval;
    let w: &World = world;
    let decisions: Vec<Decision> = (0..w.soldiers.len())
        .into_par_iter()
        .filter(|&i| {
            let s = &w.soldiers[i];
            i as u64 % interval == phase && s.alive && s.controller == Controller::Ai
        })
        .map(|i| decide(w, i))
        .collect();

    for d in &decisions {
        let s = &mut world.soldiers[d.soldier as usize];
        s.target = d.target;
        s.objective = d.objective;
        if let Some(at) = d.grenade_at {
            world.throw_grenade(d.soldier, at);
        }
    }

    // Отряды думают так же вразнобой: командир выбирает точку для всех.
    for index in 0..world.squads.len() {
        let squad = &world.squads[index];
        let Some(leader) = squad.leader.filter(|_| index as u64 % interval == phase) else { continue };
        let mut rng = Rng::for_entity(world.seed ^ 0x0050_00AD, world.tick, index as u32);
        let objective = choose_objective(&world.points, squad.team, world.soldiers[leader as usize].pos, squad.objective, world.config.defend_probability, &mut rng);
        world.squads[index].objective = objective;
        for &m in &world.squads[index].members {
            world.soldiers[m as usize].objective = objective;
        }
    }
    decisions.len() as u32
}

/// К какой точке идти из `pos`: ближайшая (с долей случайности) не наша. На своей
/// точке остаёмся с вероятностью `defend`, а если наши все — идём к любой.
fn choose_objective(points: &[CapturePoint], team: u8, pos: Vec2, current: u32, defend: f32, rng: &mut Rng) -> u32 {
    let ours = |p: &CapturePoint| p.owner == Some(team);
    let keep = match points.get(current as usize) {
        Some(p) if !ours(p) => true,
        Some(_) => rng.chance(defend),
        None => false,
    };
    if keep {
        return current;
    }
    let all_ours = points.iter().all(ours);
    let mut best = NO_OBJECTIVE;
    let mut best_score = f32::INFINITY;
    for (index, p) in points.iter().enumerate() {
        if ours(p) && !all_ours {
            continue;
        }
        let score = p.pos.distance(pos) * rng.range(0.6, 1.4);
        if score < best_score {
            best_score = score;
            best = index as u32;
        }
    }
    best
}

fn decide(w: &World, i: usize) -> Decision {
    let me = &w.soldiers[i];
    let mut rng = Rng::for_entity(w.seed, w.tick, i as u32);
    let vision = w.config.soldier.vision_range;
    let vision2 = vision * vision;

    // --- Восприятие: ближайшие враги, потом проверка прямой видимости.
    let mut target = me.target.filter(|&t| {
        let o = &w.soldiers[t as usize];
        o.alive && o.pos.distance_squared(me.pos) < vision2 && w.map.line_of_sight(me.pos, o.pos)
    });
    if target.is_none() {
        let k = (w.config.soldier.vision_candidates as usize).clamp(1, MAX_CANDIDATES);
        let mut nearest: [(f32, u32); MAX_CANDIDATES] = [(f32::INFINITY, 0); MAX_CANDIDATES];
        w.grid.query(me.pos, vision, |j| {
            let o = &w.soldiers[j as usize];
            if o.team == me.team {
                return;
            }
            let d2 = o.pos.distance_squared(me.pos);
            if d2 >= vision2 || d2 >= nearest[k - 1].0 {
                return;
            }
            // Вставка в маленький отсортированный массив.
            let mut slot = k - 1;
            while slot > 0 && nearest[slot - 1].0 > d2 {
                nearest[slot] = nearest[slot - 1];
                slot -= 1;
            }
            nearest[slot] = (d2, j);
        });
        target = nearest[..k]
            .iter()
            .take_while(|(d2, _)| d2.is_finite())
            .map(|&(_, j)| j)
            .find(|&j| w.map.line_of_sight(me.pos, w.soldiers[j as usize].pos));
    }

    // --- Цель движения. В отряде с командиром её выбирает командир (ниже, в `think`);
    // без отряда — сам: часть остаётся защищать свою точку, остальные идут дальше.
    let led = w.squads.get(me.squad as usize).is_some_and(|q| q.leader.is_some());
    let objective = if led {
        me.objective
    } else {
        choose_objective(&w.points, me.team, me.pos, me.objective, w.config.defend_probability, &mut rng)
    };

    // --- Граната: если враги сбились в кучу на подходящей дистанции.
    let mut grenade_at = None;
    let g = &w.config.grenade;
    if let Some(t) = target
        && me.grenades > 0
        && me.grenade_timer <= 0.0
    {
        let at = w.soldiers[t as usize].pos;
        let distance = at.distance(me.pos);
        if distance >= g.min_distance && distance <= g.max_distance {
            let mut cluster = 0;
            w.grid.query(at, 5.0, |j| {
                let o = &w.soldiers[j as usize];
                if o.team != me.team && o.pos.distance_squared(at) < 25.0 {
                    cluster += 1;
                }
            });
            if cluster >= g.min_enemies {
                grenade_at = Some(at + Vec2::new(rng.range(-2.0, 2.0), rng.range(-2.0, 2.0)));
            }
        }
    }

    Decision { soldier: i as u32, target, objective, grenade_at }
}
