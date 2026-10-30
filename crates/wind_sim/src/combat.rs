//! Стрельба, полёт пуль и гранат, взрывы.

use crate::arsenal::{BlastSpec, MAX_HEAT, WeaponSpec};
use crate::suppression::SuppressionSpec;
use crate::rng::Rng;
use crate::world::{BLAST_PUSH, BULLET_PUSH, Controller, Event, Projectile, ProjectileKind, Soldier, World, snapshot};
use glam::Vec2;
use rayon::prelude::*;

/// Солдаты с целью стреляют. Решения параллельно, пули собираются в общий список.
pub(crate) fn fire(world: &mut World) {
    let dt = world.dt();
    let arsenal = &world.config.arsenal;
    let radius = world.config.soldier.radius;
    let (seed, tick) = (world.seed, world.tick);
    let prev = snapshot(&world.soldiers);
    let map = &world.map;
    let sup = &world.config.suppression;

    let shots: Vec<Projectile> = world
        .soldiers
        .par_iter_mut()
        .enumerate()
        .filter_map(|(i, s)| {
            if !s.alive {
                return None;
            }
            let kit = &arsenal.teams[s.team as usize & 1][s.kit as usize];
            let weapon = &arsenal.weapons[kit.weapon];
            s.cooldown -= dt;
            s.grenade_timer -= dt;
            s.heat = (s.heat - weapon.sustained_fire_diminish_rate * dt).max(0.0);
            if s.reload_timer > 0.0 {
                s.reload_timer -= dt;
                if s.reload_timer <= 0.0 {
                    s.ammo = weapon.magazine_size;
                }
                return None;
            }
            // Игрок перезаряжается сам, когда захочет, — если магазин не полон.
            if let Controller::Player(input) = s.controller
                && input.reload
                && s.ammo < weapon.magazine_size
            {
                s.reload_timer = weapon.reload_time;
                return None;
            }
            let to = match s.controller {
                // Игрок стреляет куда целится, видимость не проверяем.
                Controller::Player(input) => {
                    if !input.fire || s.cooldown > 0.0 {
                        return None;
                    }
                    input.aim - s.pos
                }
                Controller::Ai => {
                    let Some(target) = s.target else {
                        // Огонь на подавление: врага не видно, но он только что был вон
                        // там — автоматчик бьёт туда реже обычного, пока есть патроны.
                        let spot = s.last_seen.filter(|_| {
                            s.last_seen_timer > 0.0
                                && weapon.blast.is_none()
                                && weapon.retrigger_time <= 0.2
                                && s.ammo * 3 > weapon.magazine_size
                        })?;
                        if s.cooldown > 0.0 {
                            return None;
                        }
                        let to = spot - s.pos;
                        if to.length() > weapon.range || to.length() < 3.0 || !map.line_of_sight(s.pos, spot) {
                            return None;
                        }
                        s.cooldown = weapon.retrigger_time;
                        return shoot(s, i, to, weapon, kit.weapon, radius, seed, tick, sup);
                    };
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
                    // Гранатомётом в упор не стреляют — заденет и себя.
                    if weapon.blast.as_ref().is_some_and(|b| to.length() < b.radius * 1.3) {
                        return None;
                    }
                    to
                }
            };
            if to == Vec2::ZERO {
                return None;
            }
            // Остаток паузы переносим — иначе темп упирается в тики (M4 стрелял бы раз
            // в 3 тика вместо 8,8 раза в секунду). Но не больше тика: после паузы очереди
            // «за прошлое» нет.
            s.cooldown = s.cooldown.max(-dt);
            shoot(s, i, to, weapon, kit.weapon, radius, seed, tick, sup)
        })
        .collect();
    for p in &shots {
        world.events.push(match p.kind {
            // Снаряд гранатомёта клиенту показываем как летящую гранату.
            ProjectileKind::Shell { flight } => {
                Event::GrenadeThrown { shooter: p.shooter, from: p.pos, to: p.pos + p.vel * flight, fuse: flight }
            }
            _ => Event::Shot { shooter: p.shooter, angle: p.vel.y.atan2(p.vel.x) },
        });
    }
    world.projectiles.extend(shots);

    // Гранаты игроков. ИИ бросает свои в ai::think.
    for i in 0..world.soldiers.len() {
        let s = &world.soldiers[i];
        if let Controller::Player(input) = s.controller
            && input.grenade
            && s.alive
            && s.grenades > 0
            && s.grenade_timer <= 0.0
        {
            world.throw_grenade(i as u32, input.aim);
        }
    }
}

/// Выстрел в направлении `to`: разброс (больше в очереди и под подавлением),
/// пауза до следующего, патроны, снаряд.
#[allow(clippy::too_many_arguments)]
fn shoot(s: &mut Soldier, i: usize, to: Vec2, weapon: &WeaponSpec, weapon_index: usize, radius: f32, seed: u64, tick: u64, sup: &SuppressionSpec) -> Option<Projectile> {
    let mut rng = Rng::for_entity(seed ^ 0xF17E, tick, i as u32);
    // Подавленный ИИ мажет; игрок целится сам.
    let shaken = if s.controller == Controller::Ai { 1.0 + s.suppression * sup.spread_penalty } else { 1.0 };
    let angle = to.y.atan2(to.x) + rng.range(-0.5, 0.5) * weapon.spread(s.heat) * shaken;
    let dir = Vec2::new(angle.cos(), angle.sin());
    s.heat = (s.heat + weapon.sustained_fire_grow_step).min(MAX_HEAT);
    s.cooldown += weapon.retrigger_time;
    s.ammo = s.ammo.saturating_sub(1);
    if s.ammo == 0 {
        s.reload_timer = weapon.reload_time;
    }
    let kind = match weapon.blast {
        // Снаряд летит до точки прицела, но не дальше дальности.
        Some(_) => ProjectileKind::Shell { flight: to.length().min(weapon.range) / weapon.projectile_speed },
        None => ProjectileKind::Bullet,
    };
    Some(Projectile {
        pos: s.pos + dir * (radius + 0.05),
        vel: dir * weapon.projectile_speed,
        ttl: weapon.range * 1.2 / weapon.projectile_speed,
        team: s.team,
        shooter: i as u32,
        weapon: weapon_index as u8,
        kind,
    })
}

enum Outcome {
    Flying,
    Gone,
    HitSoldier(u32),
    Explode,
}

/// Кого пуля миновала совсем рядом (не больше четырёх — хватит).
#[derive(Default)]
struct NearMisses {
    ids: [u32; 4],
    count: usize,
}

impl NearMisses {
    fn push(&mut self, id: u32) {
        if self.count < self.ids.len() {
            self.ids[self.count] = id;
            self.count += 1;
        }
    }
}

/// Расстояние от точки до отрезка.
fn distance_to_segment(a: Vec2, b: Vec2, p: Vec2) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

pub(crate) fn update_projectiles(world: &mut World) {
    let dt = world.dt();
    let radius = world.config.soldier.radius;
    let near_miss = world.config.cover.near_miss;
    let prone_target = world.config.suppression.prone_target;
    let World { projectiles, soldiers, grid, map, .. } = world;
    let (soldiers, grid, map) = (&*soldiers, &*grid, &*map);

    // Параллельно двигаем снаряды и ищем попадания, только читая мир.
    let outcomes: Vec<(Outcome, NearMisses)> = projectiles
        .par_iter_mut()
        .map(|p| match &mut p.kind {
            ProjectileKind::Bullet => {
                let next = p.pos + p.vel * dt;
                let wall_t = map.cast(p.pos, next).map_or(f32::INFINITY, |(t, _, _)| t);
                let mid = (p.pos + next) * 0.5;
                let reach = (next - p.pos).length() * 0.5 + (radius + 1.0).max(near_miss);
                let mut hit: Option<(f32, u32)> = None;
                let mut near = NearMisses::default();
                // До стены пуля долетает не вся: свист за стеной не пугает.
                let end = if wall_t <= 1.0 { p.pos + (next - p.pos) * wall_t } else { next };
                grid.query(mid, reach, |j| {
                    let s = &soldiers[j as usize];
                    if s.team == p.team || !s.alive {
                        return;
                    }
                    let body = if s.prone { radius * prone_target } else { radius };
                    if let Some(t) = segment_hits_circle(p.pos, next, s.pos, body)
                        && t <= wall_t
                        && hit.is_none_or(|(best, _)| t < best)
                    {
                        hit = Some((t, j));
                    }
                    if distance_to_segment(p.pos, end, s.pos) < near_miss {
                        near.push(j);
                    }
                });
                let outcome = if let Some((_, j)) = hit {
                    Outcome::HitSoldier(j)
                } else if wall_t <= 1.0 {
                    Outcome::Gone
                } else {
                    p.pos = next;
                    p.ttl -= dt;
                    if p.ttl <= 0.0 { Outcome::Gone } else { Outcome::Flying }
                };
                (outcome, near)
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
                (if *fuse <= 0.0 { Outcome::Explode } else { Outcome::Flying }, NearMisses::default())
            }
            ProjectileKind::Shell { flight } => {
                let next = p.pos + p.vel * dt;
                let wall_t = map.cast(p.pos, next).map_or(f32::INFINITY, |(t, _, _)| t);
                let mut hit_t = f32::INFINITY;
                let mid = (p.pos + next) * 0.5;
                let reach = (next - p.pos).length() * 0.5 + radius + 1.0;
                grid.query(mid, reach, |j| {
                    let s = &soldiers[j as usize];
                    if s.team != p.team
                        && s.alive
                        && let Some(t) = segment_hits_circle(p.pos, next, s.pos, radius)
                    {
                        hit_t = hit_t.min(t);
                    }
                });
                // Взрываемся у стены (чуть не долетев), у врага или в точке прицела.
                let t = wall_t.min(hit_t);
                *flight -= dt;
                let outcome = if t <= 1.0 {
                    p.pos += (next - p.pos) * (t - 0.05).max(0.0);
                    Outcome::Explode
                } else {
                    p.pos = next;
                    if *flight <= 0.0 { Outcome::Explode } else { Outcome::Flying }
                };
                (outcome, NearMisses::default())
            }
        })
        .collect();

    // По кому стреляли: пуля прошла рядом или попала — солдат запоминает, откуда.
    let under_fire = world.config.cover.under_fire;
    let (per_near, per_hit) = (world.config.suppression.per_near_miss, world.config.suppression.per_hit);
    for (index, (outcome, near)) in outcomes.iter().enumerate() {
        let shooter = world.projectiles[index].shooter;
        let from = world.soldiers[shooter as usize].pos;
        let hit = match *outcome {
            Outcome::HitSoldier(v) => Some(v),
            _ => None,
        };
        let near = near.ids[..near.count].iter().filter(|&&j| Some(j) != hit).map(|&j| (j, per_near));
        for (j, weight) in near.chain(hit.map(|j| (j, per_hit))) {
            let s = &mut world.soldiers[j as usize];
            s.threat = Some(from);
            s.under_fire = under_fire;
            s.suppression = (s.suppression + weight).min(1.0);
        }
    }

    // Последствия применяем по порядку — так результат не зависит от потоков.
    let mut finished = Vec::new();
    let mut explosions = Vec::new();
    for (index, (outcome, _)) in outcomes.iter().enumerate() {
        let p = &world.projectiles[index];
        match *outcome {
            Outcome::Flying => continue,
            Outcome::Gone => {}
            Outcome::HitSoldier(victim) => {
                let shooter = p.shooter;
                let mut rng = Rng::for_entity(world.seed ^ 0x417, world.tick, index as u32);
                let weapon = &world.config.arsenal.weapons[p.weapon as usize];
                let s = &mut world.soldiers[victim as usize];
                // Как в RWR: попадание убивает с некоторой вероятностью, иначе ранит.
                let killed = s.alive && {
                    s.health -= weapon.wound_damage;
                    rng.chance(weapon.kill_probability) || s.health <= 0.0
                };
                if killed {
                    world.kill(victim, shooter, p.vel.normalize_or_zero() * BULLET_PUSH, false);
                }
            }
            Outcome::Explode => {
                let blast = match p.kind {
                    ProjectileKind::Shell { .. } => world.config.arsenal.weapons[p.weapon as usize].blast.clone().unwrap_or_default(),
                    _ => world.grenade_blast(),
                };
                explosions.push((p.pos, p.shooter, index as u32, blast));
            }
        }
        finished.push(index);
    }
    for (pos, shooter, salt, blast) in explosions {
        explode(world, pos, shooter, salt, &blast);
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

fn explode(world: &mut World, pos: Vec2, shooter: u32, salt: u32, blast: &BlastSpec) {
    let mut victims = Vec::new();
    let soldiers = &world.soldiers;
    world.grid.query(pos, blast.radius, |j| {
        let s = &soldiers[j as usize];
        let d = s.pos.distance(pos);
        if s.alive && d < blast.radius && world.map.line_of_sight(pos, s.pos) {
            victims.push((j, d));
        }
    });
    let mut rng = Rng::for_entity(world.seed ^ 0xB00, world.tick, salt);
    for (j, d) in victims {
        let falloff = 1.0 - d / blast.radius;
        if rng.chance(blast.kill_probability * falloff) {
            let away = (world.soldiers[j as usize].pos - pos).normalize_or(Vec2::X);
            world.kill(j, shooter, away * BLAST_PUSH * (0.6 + 0.4 * falloff), true);
        }
    }
    world.map.damage_circle(pos, blast.wall_damage_radius, blast.wall_damage);
    world.events.push(Event::Explosion { pos, radius: blast.radius });
}

/// Задевает ли отрезок `a→b` круг. Возвращает долю пути до ближайшей к центру точки.
fn segment_hits_circle(a: Vec2, b: Vec2, center: Vec2, radius: f32) -> Option<f32> {
    let d = b - a;
    let len2 = d.length_squared();
    let t = if len2 > 0.0 { ((center - a).dot(d) / len2).clamp(0.0, 1.0) } else { 0.0 };
    ((a + d * t).distance_squared(center) < radius * radius).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::SimConfig;

    /// Средний промах по углу, радианы: стреляем 400 раз точно вправо.
    fn mean_error(suppression: f32) -> f32 {
        let w = World::new(SimConfig { map_size: 128, soldiers_per_team: 1, capture_points: 1, ..Default::default() }, 1);
        let weapon = w.weapon(0).clone();
        let sup = SuppressionSpec::default();
        let mut s = w.soldiers[0].clone();
        s.suppression = suppression;
        let mut sum = 0.0;
        for tick in 0..400 {
            s.heat = 0.0;
            s.ammo = 30;
            let p = shoot(&mut s, 0, Vec2::X, &weapon, 0, 0.35, 7, tick, &sup).unwrap();
            sum += p.vel.y.atan2(p.vel.x).abs();
        }
        sum / 400.0
    }

    #[test]
    fn suppressed_soldier_misses_more() {
        let calm = mean_error(0.0);
        let shaken = mean_error(1.0);
        // Полное подавление — разброс в 1 + spread_penalty = 2,5 раза шире.
        assert!((shaken / calm - 2.5).abs() < 0.05, "спокойный {calm}, подавленный {shaken}");
    }
}
