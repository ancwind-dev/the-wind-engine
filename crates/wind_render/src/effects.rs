//! Обломки и пыль — только картинка: симуляция о них не знает, по сети они не идут.
//! Появляются из того, что видит клиент: взрывов и клеток стен, которые стали слабее
//! (см. `MapUpdate::broken`). Поэтому одинаково работают и в своей игре, и по сети.

use crate::mapmesh::BrokenCell;
use crate::renderer::{Instance, Particle};
use glam::{Mat4, Vec2, Vec3};
use wind_sim::map::Map;

/// Сколько обломков лежит одновременно — старые уходят первыми.
const MAX_DEBRIS: usize = 3000;
const MAX_PUFFS: usize = 1500;
/// Сколько секунд лежит обломок, прежде чем уйти в землю.
const DEBRIS_LIFE: f32 = 30.0;
const SINK_TIME: f32 = 3.0;
const GRAVITY: f32 = 9.8;
/// Высота, ниже которой обломок упирается в стену.
const WALL_HEIGHT: f32 = 2.6;

struct Debris {
    pos: Vec3,
    vel: Vec3,
    size: f32,
    angle: f32,
    spin: f32,
    color: [f32; 3],
    /// Сколько секунд лежит.
    rest: f32,
    resting: bool,
}

/// Клуб пыли или дыма.
struct Puff {
    pos: Vec3,
    vel: Vec3,
    size: f32,
    grow: f32,
    age: f32,
    life: f32,
    color: [f32; 3],
    alpha: f32,
}

#[derive(Default)]
pub struct Effects {
    debris: Vec<Debris>,
    puffs: Vec<Puff>,
    /// Взрывы, от которых уже пошёл дым: (центр, когда).
    seen_blasts: Vec<(Vec2, f32)>,
    time: f32,
    seed: u32,
}

impl Effects {
    pub fn reset(&mut self) {
        *self = Effects::default();
    }

    /// Случайное число 0..1 — свой простой генератор: эффекты не обязаны совпадать
    /// между компьютерами.
    fn rand(&mut self) -> f32 {
        self.seed = self.seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (self.seed >> 8) as f32 / (1u32 << 24) as f32
    }

    fn spread(&mut self) -> f32 {
        self.rand() * 2.0 - 1.0
    }

    /// Кадр: новые взрывы дают дым и пыль, сломанные клетки — обломки и пыль цвета стены.
    pub fn update(&mut self, map: &Map, dt: f32, blasts: &[(Vec2, f32, f32)], broken: &[BrokenCell]) {
        self.time += dt;
        let now = self.time;
        self.seen_blasts.retain(|&(_, t)| now - t < 3.0);
        for &(center, radius, age) in blasts {
            if age > 0.5 || self.seen_blasts.iter().any(|&(c, _)| c.distance_squared(center) < 0.25) {
                continue;
            }
            self.seen_blasts.push((center, now));
            self.blast(map, center, radius);
        }
        for &cell in broken {
            self.crumble(map, cell);
        }
        self.step(map, dt);
    }

    fn blast(&mut self, map: &Map, center: Vec2, radius: f32) {
        let ground = map.height_at(center);
        // Серый дым поднимается столбом.
        for _ in 0..4 {
            let p = Vec3::new(center.x + self.spread() * 0.6, ground + 0.4 + self.rand(), center.y + self.spread() * 0.6);
            let vel = Vec3::new(self.spread() * 0.6, 1.2 + self.rand() * 1.2, self.spread() * 0.6);
            let (size, life) = (1.0 + self.rand() * 0.6, 2.5 + self.rand() * 1.5);
            self.puff(p, vel, size, 1.0, life, [0.42, 0.4, 0.37], 0.38);
        }
        // Пыль кольцом по земле — от центра наружу.
        for k in 0..12 {
            let a = k as f32 / 12.0 * std::f32::consts::TAU + self.spread() * 0.2;
            let dir = Vec3::new(a.cos(), 0.0, a.sin());
            let p = Vec3::new(center.x, ground + 0.3, center.y) + dir * radius * 0.3;
            let vel = dir * (radius * 1.2 + self.rand() * 2.0) + Vec3::Y * 0.3;
            let (size, life) = (0.8 + self.rand() * 0.5, 1.6 + self.rand());
            self.puff(p, vel, size, 0.8, life, [0.66, 0.61, 0.52], 0.32);
        }
    }

    fn crumble(&mut self, map: &Map, cell: BrokenCell) {
        let center = Vec2::new(cell.x as f32 + 0.5, cell.y as f32 + 0.5);
        let ground = map.height_at(center);
        // Обломки летят от ближайшего недавнего взрыва, а без него — во все стороны.
        let from = self.seen_blasts.iter().map(|&(c, _)| c).filter(|c| c.distance(center) < 12.0).min_by(|a, b| {
            a.distance_squared(center).total_cmp(&b.distance_squared(center))
        });
        let count = if cell.destroyed { 10 } else { (cell.lost * 10.0).ceil().clamp(1.0, 6.0) as usize };
        for _ in 0..count {
            let away = match from {
                Some(c) => (center - c).normalize_or_zero(),
                None => Vec2::ZERO,
            };
            let out = Vec2::new(away.x + self.spread() * 0.7, away.y + self.spread() * 0.7) * (2.0 + self.rand() * 4.0);
            let shade = 0.75 + self.rand() * 0.35;
            let debris = Debris {
                pos: Vec3::new(center.x + self.spread() * 0.4, ground + 0.4 + self.rand() * 1.8, center.y + self.spread() * 0.4),
                vel: Vec3::new(out.x, 2.0 + self.rand() * 4.0, out.y),
                size: 0.1 + self.rand() * 0.18,
                angle: self.rand() * std::f32::consts::TAU,
                spin: self.spread() * 8.0,
                color: cell.color.map(|c| c * shade),
                rest: 0.0,
                resting: false,
            };
            self.debris.push(debris);
        }
        // Пыль цвета стены.
        let dust = cell.color.map(|c| (c * 1.15 + 0.08).min(1.0));
        for _ in 0..if cell.destroyed { 3 } else { 1 } {
            let p = Vec3::new(center.x + self.spread() * 0.5, ground + 0.6 + self.rand(), center.y + self.spread() * 0.5);
            let vel = Vec3::new(self.spread(), 0.4 + self.rand() * 0.6, self.spread());
            let (size, life) = (0.7 + self.rand() * 0.5, 2.0 + self.rand() * 1.5);
            self.puff(p, vel, size, 0.6, life, dust, 0.38);
        }
        // Лишние обломки — сначала самые старые из лежащих.
        if self.debris.len() > MAX_DEBRIS {
            let extra = self.debris.len() - MAX_DEBRIS;
            self.debris.drain(..extra);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn puff(&mut self, pos: Vec3, vel: Vec3, size: f32, grow: f32, life: f32, color: [f32; 3], alpha: f32) {
        if self.puffs.len() >= MAX_PUFFS {
            self.puffs.remove(0);
        }
        self.puffs.push(Puff { pos, vel, size, grow, age: 0.0, life, color, alpha });
    }

    fn step(&mut self, map: &Map, dt: f32) {
        for d in &mut self.debris {
            if d.resting {
                d.rest += dt;
                continue;
            }
            d.vel.y -= GRAVITY * dt;
            let next = d.pos + d.vel * dt;
            // В стену не пролетаем — отскакиваем.
            if next.y < map.height_at(Vec2::new(next.x, next.z)) + WALL_HEIGHT && map.is_solid_at(Vec2::new(next.x, next.z)) {
                d.vel.x *= -0.3;
                d.vel.z *= -0.3;
            } else {
                d.pos.x = next.x;
                d.pos.z = next.z;
            }
            d.pos.y = next.y;
            d.angle += d.spin * dt;
            let floor = map.height_at(Vec2::new(d.pos.x, d.pos.z)) + d.size * 0.5;
            if d.pos.y < floor {
                d.pos.y = floor;
                d.vel.y *= -0.3;
                d.vel.x *= 0.55;
                d.vel.z *= 0.55;
                d.spin *= 0.5;
                if d.vel.length() < 0.6 {
                    d.resting = true;
                }
            }
        }
        self.debris.retain(|d| d.rest < DEBRIS_LIFE + SINK_TIME);
        for p in &mut self.puffs {
            p.age += dt;
            p.pos += p.vel * dt;
            p.vel *= (-dt * 1.6).exp();
            p.size += p.grow * dt;
        }
        self.puffs.retain(|p| p.age < p.life);
    }

    /// Обломки — кубики (меш `Cube`, освещённые, цвет из оттенка).
    pub fn debris_instances(&self, out: &mut Vec<Instance>) {
        for d in &self.debris {
            // Отлежав своё, обломок уходит в землю.
            let sink = ((d.rest - DEBRIS_LIFE) / SINK_TIME).clamp(0.0, 1.0) * d.size;
            let m = Mat4::from_translation(d.pos - Vec3::Y * sink) * Mat4::from_rotation_y(d.angle) * Mat4::from_scale(Vec3::splat(d.size));
            out.push(Instance { model: m.to_cols_array_2d(), tint: [d.color[0], d.color[1], d.color[2], 0.5] });
        }
    }

    /// Клубы пыли и дыма — от дальних к ближним (прозрачное рисуется по порядку).
    pub fn particles(&self, eye: Vec3) -> Vec<Particle> {
        let mut out: Vec<(f32, Particle)> = self
            .puffs
            .iter()
            .map(|p| {
                let t = p.age / p.life;
                // Быстро проявляется, медленно тает.
                let alpha = p.alpha * (t / 0.1).min(1.0) * (1.0 - t).powf(1.5);
                (p.pos.distance_squared(eye), Particle { pos: p.pos.to_array(), size: p.size, color: [p.color[0], p.color[1], p.color[2], alpha] })
            })
            .collect();
        out.sort_by(|a, b| b.0.total_cmp(&a.0));
        out.into_iter().map(|e| e.1).collect()
    }

}
