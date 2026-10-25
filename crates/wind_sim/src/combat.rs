//! Стрельба, полёт пуль и гранат, взрывы.

use crate::rng::Rng;
use crate::world::{Event, Projectile, ProjectileKind, World, snapshot};
use glam::Vec2;
use rayon::prelude::*;

/// Солдаты с целью стреляют. Решения параллельно, пули собираются в общий список.
pub(crate) fn fire(world: &mut World) {
    let dt = world.dt();
    let weapon = &world.config.weapon;
    let radius = world.config.soldier.radius;
    let (seed, tick) = (world.seed, world.tick);
    let prev = snapshot(&world.soldiers);
    let map = &world.map;
    let spread = weapon.spread_deg.to_radians();

    let shots: Vec<Projectile> = world
        .soldiers
        .par_iter_mut()
        .enumerate()
        .filter_map(|(i, s)| {
            if !s.alive {
                return None;
            }
            s.cooldown -= dt;
            s.grenade_timer -= dt;
            if s.reload_timer > 0.0 {
                s.reload_timer -= dt;
                if s.reload_timer <= 0.0 {
                    s.ammo = weapon.magazine_size;
                }
                return None;
            }
            let target = s.target?;
            let (target_pos, target_alive) = prev[target as usize];
            if !target_alive {
                s.target = None;
                return None;
            }
            if s.cooldown > 0.0 {
                return None;
            }
            let to = target_pos - s.pos;
            if to.length() > weapon.range {
                return None;
            }
            if !map.line_of_sight(s.pos, target_pos) {
                s.target = None;
                return None;
            }
            let mut rng = Rng::for_entity(seed ^ 0xF17E, tick, i as u32);
            let angle = to.y.atan2(to.x) + rng.range(-0.5, 0.5) * spread;
            let dir = Vec2::new(angle.cos(), angle.sin());
            s.cooldown += weapon.retrigger_time;
            s.ammo = s.ammo.saturating_sub(1);
            if s.ammo == 0 {
                s.reload_timer = weapon.reload_time;
            }
            Some(Projectile {
                pos: s.pos + dir * (radius + 0.05),
                vel: dir * weapon.projectile_speed,
                ttl: weapon.range * 1.2 / weapon.projectile_speed,
                team: s.team,
                shooter: i as u32,
                kind: ProjectileKind::Bullet,
            })
        })
        .collect();
    world.projectiles.extend(shots);
}

enum Outcome {
    Flying,
    Gone,
    HitSoldier(u32),
    Explode,
}

pub(crate) fn update_projectiles(world: &mut World) {
    let dt = world.dt();
    let radius = world.config.soldier.radius;
    let World { projectiles, soldiers, grid, map, .. } = world;
    let (soldiers, grid, map) = (&*soldiers, &*grid, &*map);

    // Параллельно двигаем снаряды и ищем попадания, только читая мир.
    let outcomes: Vec<Outcome> = projectiles
        .par_iter_mut()
        .map(|p| match &mut p.kind {
            ProjectileKind::Bullet => {
                let next = p.pos + p.vel * dt;
                let wall_t = map.cast(p.pos, next).map_or(f32::INFINITY, |(t, _, _)| t);
                let mid = (p.pos + next) * 0.5;
                let reach = (next - p.pos).length() * 0.5 + radius + 1.0;
                let mut hit: Option<(f32, u32)> = None;
                grid.query(mid, reach, |j| {
                    let s = &soldiers[j as usize];
                    if s.team == p.team || !s.alive {
                        return;
                    }
                    if let Some(t) = segment_hits_circle(p.pos, next, s.pos, radius)
                        && t <= wall_t
                        && hit.is_none_or(|(best, _)| t < best)
                    {
                        hit = Some((t, j));
                    }
                });
                if let Some((_, j)) = hit {
                    Outcome::HitSoldier(j)
                } else if wall_t <= 1.0 {
                    Outcome::Gone
                } else {
                    p.pos = next;
                    p.ttl -= dt;
                    if p.ttl <= 0.0 { Outcome::Gone } else { Outcome::Flying }
                }
            }
            ProjectileKind::Grenade { fuse, flight } => {
                *fuse -= dt;
                if *flight > 0.0 {
                    *flight -= dt;
                    let step = p.vel * dt;
                    // Отскок от стены по той оси, по которой врезались.
                    if map.is_solid_at(p.pos + Vec2::new(step.x, 0.0)) {
                        p.vel.x = -p.vel.x * 0.4;
                    } else {
                        p.pos.x += step.x;
                    }
                    if map.is_solid_at(p.pos + Vec2::new(0.0, step.y)) {
                        p.vel.y = -p.vel.y * 0.4;
                    } else {
                        p.pos.y += step.y;
                    }
                }
                if *fuse <= 0.0 { Outcome::Explode } else { Outcome::Flying }
            }
        })
        .collect();

    // Последствия применяем по порядку — так результат не зависит от потоков.
    let mut finished = Vec::new();
    let mut explosions = Vec::new();
    for (index, outcome) in outcomes.iter().enumerate() {
        let p = &world.projectiles[index];
        match *outcome {
            Outcome::Flying => continue,
            Outcome::Gone => {}
            Outcome::HitSoldier(victim) => {
                let shooter = p.shooter;
                let mut rng = Rng::for_entity(world.seed ^ 0x417, world.tick, index as u32);
                let weapon = &world.config.weapon;
                let s = &mut world.soldiers[victim as usize];
                // Как в RWR: попадание убивает с некоторой вероятностью, иначе ранит.
                let killed = s.alive && {
                    s.health -= weapon.wound_damage;
                    rng.chance(weapon.kill_probability) || s.health <= 0.0
                };
                if killed {
                    world.kill(victim, shooter);
                }
            }
            Outcome::Explode => explosions.push((p.pos, p.shooter, index as u32)),
        }
        finished.push(index);
    }
    for (pos, shooter, salt) in explosions {
        explode(world, pos, shooter, salt);
    }
    // Удаляем отработавшие снаряды, сохраняя порядок остальных.
    let mut keep = vec![true; world.projectiles.len()];
    for i in finished {
        keep[i] = false;
    }
    let mut i = 0;
    world.projectiles.retain(|_| {
        i += 1;
        keep[i - 1]
    });
}

fn explode(world: &mut World, pos: Vec2, shooter: u32, salt: u32) {
    let g = world.config.grenade.clone();
    let mut victims = Vec::new();
    let soldiers = &world.soldiers;
    world.grid.query(pos, g.blast_radius, |j| {
        let s = &soldiers[j as usize];
        let d = s.pos.distance(pos);
        if s.alive && d < g.blast_radius && world.map.line_of_sight(pos, s.pos) {
            victims.push((j, d));
        }
    });
    let mut rng = Rng::for_entity(world.seed ^ 0xB00, world.tick, salt);
    for (j, d) in victims {
        let falloff = 1.0 - d / g.blast_radius;
        if rng.chance(g.kill_probability * falloff) {
            world.kill(j, shooter);
        }
    }
    world.map.damage_circle(pos, g.wall_damage_radius, g.wall_damage);
    world.events.push(Event::Explosion { pos, radius: g.blast_radius });
}

/// Задевает ли отрезок `a→b` круг. Возвращает долю пути до ближайшей к центру точки.
fn segment_hits_circle(a: Vec2, b: Vec2, center: Vec2, radius: f32) -> Option<f32> {
    let d = b - a;
    let len2 = d.length_squared();
    let t = if len2 > 0.0 { ((center - a).dot(d) / len2).clamp(0.0, 1.0) } else { 0.0 };
    ((a + d * t).distance_squared(center) < radius * radius).then_some(t)
}
