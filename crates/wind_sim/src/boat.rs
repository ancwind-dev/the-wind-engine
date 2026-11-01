//! Лодки. Стоят на воде у баз; садятся и выходят на V (`PlayerInput::use_boat`).
//! Первый севший — водитель: правит WASD (лодка разворачивается к направлению и набирает
//! ход), не стреляет. Остальные — пассажиры, стреляют как обычно. На мель лодка не идёт,
//! от взрыва рядом тонет — сидящие оказываются в воде, а лодка через `RESPAWN` с снова
//! у своей базы.

use crate::map::{Map, SWIM_DEPTH};
use crate::world::{Controller, World};
use glam::Vec2;

/// Солдат не в лодке; место в лодке свободно.
pub const NO_BOAT: u32 = u32::MAX;
pub const SEATS: usize = 4;
/// Места в лодке (x — вперёд, y — вправо), м. Первое — водителя, у мотора на корме.
pub const SEAT_AT: [Vec2; SEATS] = [Vec2::new(-1.15, 0.0), Vec2::new(-0.3, -0.45), Vec2::new(-0.3, 0.45), Vec2::new(0.9, 0.0)];
/// Длина и ширина лодки, м.
pub const LENGTH: f32 = 4.3;
pub const WIDTH: f32 = 1.8;
/// Мельче этого лодка садится на мель, м.
pub const DRAFT: f32 = 0.45;
/// Дальше этого от лодки в неё не сесть, м.
pub const BOARD_RANGE: f32 = 3.5;
const MAX_SPEED: f32 = 9.0;
const REVERSE: f32 = 3.0;
const ACCEL: f32 = 5.0;
/// Поворот, рад/с (на ходу; стоя — вдвое медленнее).
const TURN: f32 = 1.6;
/// Взрыв ближе этого к лодке её топит, м.
const SINK_RADIUS: f32 = 3.5;
/// Через сколько секунд утонувшая лодка снова у базы.
const RESPAWN: f32 = 30.0;
/// Сколько секунд после «сесть/выйти» не принимать следующее: клиент держит нажатие.
const USE_LOCK: f32 = 0.4;

#[derive(Clone, Debug)]
pub struct Boat {
    pub pos: Vec2,
    /// Куда смотрит нос, рад.
    pub heading: f32,
    /// Скорость вдоль носа, м/с (назад — меньше нуля).
    pub speed: f32,
    /// Кто на каком месте (`NO_BOAT` — пусто); место 0 — водитель.
    pub seats: [u32; SEATS],
    /// Где лодка стоит в начале раунда и после того, как утонула.
    pub home: (Vec2, f32),
    /// Утонула: сколько секунд до возвращения (0 — на воде).
    pub sunk: f32,
}

impl Boat {
    pub fn forward(&self) -> Vec2 {
        Vec2::from_angle(self.heading)
    }

    pub fn afloat(&self) -> bool {
        self.sunk <= 0.0
    }

    /// Где место `k` в мире.
    pub fn seat_pos(&self, k: usize) -> Vec2 {
        let f = self.forward();
        self.pos + f * SEAT_AT[k].x + f.perp() * SEAT_AT[k].y
    }
}

/// Помещается ли лодка здесь: под носом, кормой и бортами хватает воды и нет стен.
pub fn hull_fits(map: &Map, pos: Vec2, heading: f32) -> bool {
    let f = Vec2::from_angle(heading);
    let side = f.perp();
    [(0.5, 0.0), (-0.5, 0.0), (0.0, 0.5), (0.0, -0.5), (0.3, 0.4), (0.3, -0.4), (-0.3, 0.4), (-0.3, -0.4), (0.0, 0.0)].iter().all(|&(a, b)| {
        let p = pos + f * a * LENGTH + side * b * WIDTH;
        map.water_depth(p) >= DRAFT && !map.is_solid_at(p)
    })
}

impl World {
    /// Лодки у баз: на ближайшей к базе воде, где помещаются; носом от базы. По одной
    /// на команду, на больших картах — по две. Нет воды — нет лодок.
    pub(crate) fn place_boats(&mut self) {
        self.boats.clear();
        if self.map.water.is_none() {
            return;
        }
        let per_team = if self.map.width >= 512 { 2 } else { 1 };
        // Клетки, где лодке хватает глубины с запасом.
        let mut spots: Vec<Vec2> = Vec::new();
        for y in 0..self.map.height as i32 {
            for x in 0..self.map.width as i32 {
                let p = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                if self.map.water_depth(p) >= DRAFT + 0.3 {
                    spots.push(p);
                }
            }
        }
        for team in 0..2 {
            let base = self.bases[team];
            spots.sort_by(|a, b| a.distance_squared(base).total_cmp(&b.distance_squared(base)));
            let mut placed = 0;
            for &p in &spots {
                if placed == per_team {
                    break;
                }
                let heading = (p - base).to_angle();
                if !hull_fits(&self.map, p, heading) || self.boats.iter().any(|b| b.pos.distance(p) < LENGTH * 1.6) {
                    continue;
                }
                self.boats.push(Boat { pos: p, heading, speed: 0.0, seats: [NO_BOAT; SEATS], home: (p, heading), sunk: 0.0 });
                placed += 1;
            }
        }
    }

    /// «Сесть/выйти» игроков (V): рядом с лодкой — сесть (первым — водителем), в лодке —
    /// выйти на берег рядом.
    pub(crate) fn use_boats(&mut self) {
        let dt = self.dt();
        for i in 0..self.soldiers.len() {
            self.soldiers[i].boat_lock -= dt;
            let s = &self.soldiers[i];
            let Controller::Player(input) = s.controller else { continue };
            if !s.alive || !input.use_boat || s.boat_lock > 0.0 {
                continue;
            }
            if s.boat != NO_BOAT {
                self.leave_boat(i as u32);
            } else {
                let near = (0..self.boats.len())
                    .filter(|&b| self.boats[b].afloat() && self.boats[b].seats.contains(&NO_BOAT))
                    .min_by(|&a, &b| self.boats[a].pos.distance(s.pos).total_cmp(&self.boats[b].pos.distance(s.pos)))
                    .filter(|&b| self.boats[b].pos.distance(s.pos) < BOARD_RANGE);
                let Some(b) = near else { continue };
                let seat = self.boats[b].seats.iter().position(|&o| o == NO_BOAT).unwrap_or(0);
                self.boats[b].seats[seat] = i as u32;
                let pos = self.boats[b].seat_pos(seat);
                let s = &mut self.soldiers[i];
                s.boat = b as u32;
                s.pos = pos;
                s.cover = None;
            }
            self.soldiers[i].boat_lock = USE_LOCK;
        }
    }

    /// Выйти из лодки: на сушу или мелководье рядом, а если некуда — в воду у борта.
    pub(crate) fn leave_boat(&mut self, soldier: u32) {
        let s = &self.soldiers[soldier as usize];
        let Some(boat) = self.boats.get_mut(s.boat as usize) else { return };
        for seat in boat.seats.iter_mut().filter(|o| **o == soldier) {
            *seat = NO_BOAT;
        }
        let (center, radius) = (boat.pos, self.config.soldier.radius);
        // Ближайшее место, где можно стоять, — по кругам вокруг лодки.
        let mut best: Option<(f32, Vec2)> = None;
        for r in [1.6, 2.4, 3.2, 4.2, 5.4] {
            for k in 0..16 {
                let p = center + Vec2::from_angle(k as f32 / 16.0 * std::f32::consts::TAU) * r;
                let depth = self.map.water_depth(p);
                if depth < SWIM_DEPTH && !self.map.circle_hits_solid(p, radius) && best.is_none_or(|(d, _)| depth < d) {
                    best = Some((depth, p));
                }
            }
            if best.is_some() {
                break;
            }
        }
        let s = &mut self.soldiers[soldier as usize];
        s.boat = NO_BOAT;
        s.pos = best.map_or(center + Vec2::from_angle(s.facing).perp() * 1.4, |b| b.1);
    }

    /// Лодки плывут: водитель правит, сидящие едут вместе с лодкой. Утонувшие ждут
    /// и возвращаются домой.
    pub(crate) fn move_boats(&mut self) {
        let dt = self.dt();
        for b in 0..self.boats.len() {
            if !self.boats[b].afloat() {
                self.boats[b].sunk -= dt;
                if self.boats[b].afloat() {
                    let (pos, heading) = self.boats[b].home;
                    let boat = &mut self.boats[b];
                    (boat.pos, boat.heading, boat.speed) = (pos, heading, 0.0);
                }
                continue;
            }
            let driver = self.boats[b].seats[0];
            let input = self.soldiers.get(driver as usize).and_then(|s| match s.controller {
                Controller::Player(input) => Some(input),
                Controller::Ai => None,
            });
            let boat = &mut self.boats[b];
            // Правит: разворачивается к направлению WASD и набирает ход, если нос уже
            // смотрит туда; назад — задний ход.
            let mut target = 0.0;
            if let Some(input) = input.filter(|i| i.move_dir.length_squared() > 0.01) {
                let want = input.move_dir.to_angle();
                let diff = (want - boat.heading + std::f32::consts::PI).rem_euclid(std::f32::consts::TAU) - std::f32::consts::PI;
                if diff.abs() > 2.6 {
                    // Почти прямо назад — задний ход, не разворачиваясь.
                    target = -REVERSE * input.move_dir.length();
                } else {
                    let turn = TURN * if boat.speed.abs() > 1.0 { 1.0 } else { 0.5 };
                    boat.heading += diff.clamp(-turn * dt, turn * dt);
                    target = MAX_SPEED * input.move_dir.length() * diff.cos().max(0.0);
                }
            }
            boat.speed += (target - boat.speed).clamp(-ACCEL * dt * 1.5, ACCEL * dt);
            let next = boat.pos + boat.forward() * boat.speed * dt;
            if hull_fits(&self.map, next, boat.heading) {
                boat.pos = next;
            } else {
                // Мель или берег: стоп.
                boat.speed = 0.0;
            }
            // Сидящие — на своих местах, смотрят куда целятся (водитель — вперёд).
            let boat = self.boats[b].clone();
            for (k, &o) in boat.seats.iter().enumerate() {
                let Some(s) = self.soldiers.get_mut(o as usize) else { continue };
                s.pos = boat.seat_pos(k);
                s.vel = boat.forward() * boat.speed;
                s.facing = match s.controller {
                    Controller::Player(input) if k > 0 => (input.aim - s.pos).to_angle(),
                    _ => boat.heading,
                };
            }
        }
    }

    /// Взрыв в `at`: лодки рядом тонут, сидящие — в воду.
    pub(crate) fn sink_boats(&mut self, at: Vec2) {
        for b in 0..self.boats.len() {
            if !self.boats[b].afloat() || self.boats[b].pos.distance(at) > SINK_RADIUS {
                continue;
            }
            for o in self.boats[b].seats {
                if o != NO_BOAT {
                    self.leave_boat(o);
                }
            }
            self.boats[b].sunk = RESPAWN;
            self.boats[b].speed = 0.0;
        }
    }
}
