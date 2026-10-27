//! ИИ солдат: кого видим, куда идём, бросать ли гранату.
//!
//! Солдат думает не каждый тик, а раз в `think_interval` тиков, и все — вразнобой.
//! Сами решения считаются параллельно: каждый поток только читает мир.
//! К какой точке идти, решает командир отряда — за весь отряд (см. `squad`).
//! Под огнём солдат ищет угол, из-за которого стрелять (см. `cover`).

use crate::cover::{self, Cover, Query};
use crate::map::Map;
use crate::rng::Rng;
use crate::squad::Order;
use crate::world::{CapturePoint, Controller, NO_OBJECTIVE, World};
use glam::Vec2;
use rayon::prelude::*;
use std::collections::HashMap;

struct Decision {
    soldier: u32,
    target: Option<u32>,
    objective: u32,
    grenade_at: Option<Vec2>,
    cover: Option<Cover>,
    /// Искали новое укрытие (дорого — следующий поиск не скоро).
    searched: bool,
    /// Враг рядом и укрытие держит — держаться его дальше.
    refresh: bool,
}

/// Больше кандидатов на видимость не проверяем: самые дальние всё равно не важны.
const MAX_CANDIDATES: usize = 8;

/// Возвращает, сколько солдат подумало.
pub(crate) fn think(world: &mut World) -> u32 {
    let interval = world.config.think_interval.max(1) as u64;
    let phase = world.tick % interval;
    let w: &World = world;
    // Чьи углы уже заняты: двое в одном углу толкаются и оба на виду.
    let taken: HashMap<(i32, i32), u32> = w
        .soldiers
        .iter()
        .enumerate()
        .filter_map(|(i, s)| Some((Map::cell_of(s.cover?.shoot), i as u32)))
        .collect();
    let decisions: Vec<Decision> = (0..w.soldiers.len())
        .into_par_iter()
        .filter(|&i| {
            let s = &w.soldiers[i];
            i as u64 % interval == phase && s.alive && s.controller == Controller::Ai
        })
        .map(|i| decide(w, i, &taken))
        .collect();

    let spec = world.config.cover.clone();
    let mut claimed = HashMap::new();
    for d in &decisions {
        let s = &mut world.soldiers[d.soldier as usize];
        s.target = d.target;
        s.objective = d.objective;
        let mut cover = d.cover;
        if d.searched {
            // Не нашли — следующий поиск не скоро; нашли — нового не понадобится.
            s.cover_search = if cover.is_some() { 0.5 } else { spec.research };
            // Двое в этом тике нашли один угол — второй поищет снова чуть позже.
            if let Some(c) = cover
                && claimed.insert(Map::cell_of(c.shoot), d.soldier).is_some()
            {
                cover = None;
                s.cover_search = 0.5;
            }
        }
        s.cover = cover;
        if d.refresh && cover.is_some() {
            s.cover_timer = spec.hold;
        }
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

fn decide(w: &World, i: usize, taken: &HashMap<(i32, i32), u32>) -> Decision {
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

    // --- Укрытие: от врага, которого видим, или от того, кто по нам стрелял.
    let spec = &w.config.cover;
    let threat = target.map(|t| w.soldiers[t as usize].pos).or(me.threat.filter(|_| me.under_fire > 0.0));
    let mut cover = me.cover;
    let (mut searched, mut refresh) = (false, false);
    if let Some(threat) = threat {
        // Боец отряда ищет укрытие у своего места в строю, остальные — где стоят.
        let around = match w.orders.get(i) {
            Some(Order::Follow { slot, .. }) if slot.distance(me.pos) < w.config.squad.leash => *slot,
            _ => me.pos,
        };
        // Враг перебегает — угол держим, пока за ним можно спрятаться; стрелять
        // из него можно и по другим.
        if cover.is_some_and(|c| c.shoot.distance(around) <= spec.search_radius * 1.5 && c.shelters(&w.map, threat)) {
            cover = cover.map(|c| Cover { threat, ..c });
            refresh = true;
        } else {
            cover = None;
            if spec.search_radius > 0.0 && me.cover_search <= 0.0 {
                searched = true;
                let weapon = w.weapon(i as u32);
                let q = Query {
                    from: me.pos,
                    around,
                    threat,
                    min_range: weapon.blast.as_ref().map_or(3.0, |b| b.radius * 1.3),
                    max_range: weapon.range * 0.95,
                    radius: spec.search_radius,
                    body: w.config.soldier.radius,
                };
                cover = cover::find(&w.map, &q, |x, y| taken.get(&(x, y)).is_some_and(|&o| o != i as u32));
                refresh = cover.is_some();
            }
        }
    }

    Decision { soldier: i as u32, target, objective, grenade_at, cover, searched, refresh }
}
