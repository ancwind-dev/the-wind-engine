//! Мир и тик симуляции.

use crate::config::SimConfig;
use crate::grid::SpatialGrid;
use crate::map::Map;
use crate::atmosphere::Atmosphere;
use crate::mapfile::MapFile;
use crate::mapgen::{self, Layout};
use crate::nav::FlowField;
use crate::rng::Rng;
use crate::{ai, combat};
use glam::Vec2;
use rayon::prelude::*;
use std::time::Instant;

/// Солдат не идёт ни к какой точке (выберет на следующем «обдумывании»).
pub const NO_OBJECTIVE: u32 = u32::MAX;

/// Размер клетки пространственной сетки, м.
const GRID_CELL: f32 = 8.0;

/// Ввод игрока за один тик.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PlayerInput {
    /// Куда идти; длина не больше 1.
    pub move_dir: Vec2,
    /// Точка мира, куда смотрит прицел.
    pub aim: Vec2,
    pub fire: bool,
    pub grenade: bool,
}

/// Кто управляет солдатом.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Controller {
    Ai,
    Player(PlayerInput),
}

#[derive(Clone, Debug)]
pub struct Soldier {
    pub controller: Controller,
    pub pos: Vec2,
    pub vel: Vec2,
    /// Куда смотрит, радианы.
    pub facing: f32,
    pub team: u8,
    pub alive: bool,
    pub health: f32,
    pub respawn_timer: f32,
    /// Враг, по которому стреляет.
    pub target: Option<u32>,
    /// Индекс точки захвата, к которой идёт.
    pub objective: u32,
    pub cooldown: f32,
    pub ammo: u32,
    pub reload_timer: f32,
    pub grenades: u32,
    pub grenade_timer: f32,
    pub kills: u32,
}

impl Soldier {
    /// Где рисовать солдата через `ahead` секунд после тика: симуляция идёт 20 раз
    /// в секунду, а кадров больше — без этого солдат движется рывками, а анимация
    /// шага видит его стоящим в большинстве кадров.
    pub fn drawn_pos(&self, ahead: f32) -> Vec2 {
        self.pos + self.vel * ahead
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ProjectileKind {
    Bullet,
    /// `flight` — сколько ещё лететь, потом граната лежит до взрыва.
    Grenade { fuse: f32, flight: f32 },
}

#[derive(Clone, Debug)]
pub struct Projectile {
    pub pos: Vec2,
    pub vel: Vec2,
    pub ttl: f32,
    pub team: u8,
    pub shooter: u32,
    pub kind: ProjectileKind,
}

#[derive(Clone, Debug)]
pub struct CapturePoint {
    pub pos: Vec2,
    pub owner: Option<u8>,
    /// От -1 (команда 0) до +1 (команда 1).
    pub progress: f32,
}

/// Что произошло за тик. Для эффектов, ленты убийств и (позже) сети.
#[derive(Clone, Debug)]
pub enum Event {
    Kill { killer: u32, victim: u32 },
    Explosion { pos: Vec2, radius: f32 },
    PointCaptured { point: u32, team: u8 },
    /// Выстрел. Пули по сети не передаём — клиенту хватает этого события.
    Shot { shooter: u32, angle: f32 },
    GrenadeThrown { shooter: u32, from: Vec2, to: Vec2, fuse: f32 },
}

/// Сколько миллисекунд заняла каждая фаза тика.
#[derive(Clone, Debug, Default)]
pub struct TickStats {
    pub total_ms: f32,
    pub grid_ms: f32,
    pub nav_ms: f32,
    pub think_ms: f32,
    pub move_ms: f32,
    pub fire_ms: f32,
    pub projectiles_ms: f32,
    pub capture_ms: f32,
    /// Сколько солдат думали в этом тике.
    pub thinkers: u32,
    pub bullets: u32,
    pub grenades: u32,
    pub alive: [u32; 2],
}

pub struct World {
    pub config: SimConfig,
    pub seed: u64,
    pub tick: u64,
    pub map: Map,
    pub bases: [Vec2; 2],
    pub points: Vec<CapturePoint>,
    /// Поле путей к каждой точке захвата, по индексу точки.
    pub flow: Vec<FlowField>,
    pub soldiers: Vec<Soldier>,
    pub projectiles: Vec<Projectile>,
    /// События последнего тика.
    pub events: Vec<Event>,
    /// Убийства за матч по командам.
    pub score: [u32; 2],
    pub stats: TickStats,
    /// Карта из файла: имя и отпечаток. `None` — карта сгенерирована из seed.
    pub map_file: Option<(String, u64)>,
    /// Атмосфера карты — только для картинки.
    pub atmosphere: Atmosphere,
    pub(crate) grid: SpatialGrid,
}

impl World {
    /// Мир на процедурной карте из `config.map_size` и `seed`.
    pub fn new(config: SimConfig, seed: u64) -> Self {
        let (map, layout) = mapgen::generate(config.map_size, config.capture_points, seed);
        Self::with_map(config, seed, map, layout)
    }

    /// Мир на карте из файла (или из редактора).
    pub fn from_file(config: SimConfig, seed: u64, file: MapFile) -> Self {
        let id = (file.name.clone(), file.fingerprint());
        let atmosphere = file.atmosphere;
        let mut world = Self::with_map(config, seed, file.map, file.layout);
        world.map_file = Some(id);
        world.atmosphere = atmosphere;
        world
    }

    fn with_map(mut config: SimConfig, seed: u64, map: Map, layout: Layout) -> Self {
        let Layout { bases, capture_points } = layout;
        // Размер и число точек берём у карты, а не из настроек.
        config.map_size = map.width;
        config.capture_points = capture_points.len() as u32;
        let mut flow: Vec<FlowField> = capture_points.iter().map(|&p| FlowField::new(p)).collect();
        flow.par_iter_mut().for_each(|f| f.compute(&map));

        let points = capture_points
            .iter()
            .map(|&pos| CapturePoint { pos, owner: None, progress: 0.0 })
            .collect();
        let grid = SpatialGrid::new(map.width.max(map.height) as f32, GRID_CELL);

        let mut world = World {
            config,
            seed,
            tick: 0,
            map_file: None,
            atmosphere: Atmosphere::default(),
            map,
            bases,
            points,
            flow,
            soldiers: Vec::new(),
            projectiles: Vec::new(),
            events: Vec::new(),
            score: [0; 2],
            stats: TickStats::default(),
            grid,
        };

        let per_team = world.config.soldiers_per_team;
        let mut rng = Rng::new(seed ^ 0x5EED);
        for team in 0..2u8 {
            for _ in 0..per_team {
                let spread = (per_team as f32).sqrt() * 0.8;
                let pos = world.find_spawn_spot(world.bases[team as usize], spread, &mut rng);
                let mut soldier = world.fresh_soldier(team, pos);
                // Чтобы не стреляли все в один тик.
                soldier.cooldown = rng.range(0.0, 1.0);
                world.soldiers.push(soldier);
            }
        }
        world
    }

    pub fn dt(&self) -> f32 {
        self.config.dt()
    }

    pub fn step(&mut self) {
        let start = Instant::now();
        self.events.clear();
        let mut stats = TickStats::default();

        let t = Instant::now();
        self.rebuild_grid();
        stats.grid_ms = ms(t);

        let t = Instant::now();
        self.update_navigation();
        stats.nav_ms = ms(t);

        let t = Instant::now();
        stats.thinkers = ai::think(self);
        stats.think_ms = ms(t);

        let t = Instant::now();
        self.move_soldiers();
        stats.move_ms = ms(t);

        let t = Instant::now();
        combat::fire(self);
        stats.fire_ms = ms(t);

        let t = Instant::now();
        combat::update_projectiles(self);
        stats.projectiles_ms = ms(t);

        let t = Instant::now();
        self.update_capture_points();
        self.respawn();
        stats.capture_ms = ms(t);

        for p in &self.projectiles {
            match p.kind {
                ProjectileKind::Bullet => stats.bullets += 1,
                ProjectileKind::Grenade { .. } => stats.grenades += 1,
            }
        }
        for s in &self.soldiers {
            if s.alive {
                stats.alive[s.team as usize] += 1;
            }
        }

        self.tick += 1;
        stats.total_ms = ms(start);
        self.stats = stats;
    }

    fn rebuild_grid(&mut self) {
        let soldiers = &self.soldiers;
        self.grid.rebuild(soldiers.len(), |i| soldiers[i].alive.then_some(soldiers[i].pos));
    }

    /// После разрушений поля путей устаревают. Пересчитываем не больше одного
    /// поля за тик, чтобы взрыв не давал рывка.
    fn update_navigation(&mut self) {
        let map = &self.map;
        if let Some(field) = self.flow.iter_mut().find(|f| f.is_stale(map)) {
            field.compute(map);
        }
    }

    fn move_soldiers(&mut self) {
        let dt = self.dt();
        let spec = &self.config.soldier;
        let range = self.config.weapon.range;
        let capture_radius = self.config.capture.radius;
        let (seed, tick) = (self.seed, self.tick);
        let prev = snapshot(&self.soldiers);
        let map = &self.map;
        let flow = &self.flow;
        let grid = &self.grid;
        let radius = spec.radius;

        self.soldiers.par_iter_mut().enumerate().for_each(|(i, s)| {
            if !s.alive {
                return;
            }
            let mut desired = Vec2::ZERO;
            let mut speed = spec.speed;

            if let Controller::Player(input) = s.controller {
                desired = input.move_dir.clamp_length_max(1.0);
                let look = input.aim - s.pos;
                if look != Vec2::ZERO {
                    s.facing = look.y.atan2(look.x);
                }
            } else if let Some(target) = s.target.filter(|&t| prev[t as usize].1) {
                // Враг виден: подходим на дистанцию огня и стоим.
                let to = prev[target as usize].0 - s.pos;
                let d = to.length();
                s.facing = to.y.atan2(to.x);
                if d > range * 0.8 {
                    desired = to / d;
                    speed *= 0.7;
                }
            } else if let Some(field) = flow.get(s.objective as usize) {
                let to_objective = field.target - s.pos;
                if to_objective.length() > capture_radius * 0.5 {
                    let dir = field.direction(map, s.pos).unwrap_or_else(|| to_objective.normalize_or_zero());
                    // У каждого солдата своё боковое смещение — иначе все идут
                    // по одной линии поля путей гуськом.
                    let lane = (i as u32).wrapping_mul(2_654_435_761) >> 16;
                    let lane = lane as f32 / 65_535.0 * 2.0 - 1.0;
                    desired = (dir + dir.perp() * lane * 0.4).normalize_or_zero();
                } else {
                    // На точке: медленно бродим, меняя направление раз в 2 секунды.
                    let mut rng = Rng::for_entity(seed, tick / 40, i as u32);
                    let a = rng.range(0.0, std::f32::consts::TAU);
                    desired = Vec2::new(a.cos(), a.sin()) * 0.3;
                }
                if desired != Vec2::ZERO {
                    s.facing = desired.y.atan2(desired.x);
                }
            }

            // Расталкиваемся, чтобы не стоять друг в друге.
            let mut push = Vec2::ZERO;
            let min_dist = radius * 2.0;
            grid.query(s.pos, min_dist, |j| {
                if j as usize == i {
                    return;
                }
                let d = s.pos - prev[j as usize].0;
                let len2 = d.length_squared();
                if len2 < min_dist * min_dist && len2 > 1e-6 {
                    let len = len2.sqrt();
                    push += d / len * (min_dist - len);
                }
            });

            s.vel = desired * speed + push * 4.0;
            let step = s.vel * dt;
            // Двигаемся по осям отдельно — так солдат скользит вдоль стены.
            let nx = s.pos + Vec2::new(step.x, 0.0);
            if !map.circle_hits_solid(nx, radius) {
                s.pos = nx;
            }
            let ny = s.pos + Vec2::new(0.0, step.y);
            if !map.circle_hits_solid(ny, radius) {
                s.pos = ny;
            }
        });
    }

    fn update_capture_points(&mut self) {
        let dt = self.dt();
        let spec = &self.config.capture;
        for (index, point) in self.points.iter_mut().enumerate() {
            let mut count = [0u32; 2];
            let soldiers = &self.soldiers;
            self.grid.query(point.pos, spec.radius, |j| {
                let s = &soldiers[j as usize];
                if s.alive && s.pos.distance_squared(point.pos) < spec.radius * spec.radius {
                    count[s.team as usize] += 1;
                }
            });
            let rate = dt / spec.time.max(0.1);
            if count[0] > 0 && count[1] == 0 {
                point.progress -= rate * count[0].min(3) as f32;
            } else if count[1] > 0 && count[0] == 0 {
                point.progress += rate * count[1].min(3) as f32;
            } else if count == [0, 0] {
                // Никого нет — точка медленно возвращается к хозяину.
                let rest = match point.owner {
                    Some(0) => -1.0,
                    Some(_) => 1.0,
                    None => 0.0,
                };
                point.progress += (rest - point.progress).clamp(-rate * 0.5, rate * 0.5);
            }
            point.progress = point.progress.clamp(-1.0, 1.0);

            let new_owner = if point.progress <= -1.0 {
                Some(0)
            } else if point.progress >= 1.0 {
                Some(1)
            } else {
                match point.owner {
                    Some(0) if point.progress > 0.0 => None,
                    Some(1) if point.progress < 0.0 => None,
                    owner => owner,
                }
            };
            if new_owner != point.owner {
                point.owner = new_owner;
                if let Some(team) = new_owner {
                    self.events.push(Event::PointCaptured { point: index as u32, team });
                }
            }
        }
    }

    fn respawn(&mut self) {
        let dt = self.dt();
        for i in 0..self.soldiers.len() {
            let s = &mut self.soldiers[i];
            if s.alive {
                continue;
            }
            s.respawn_timer -= dt;
            if s.respawn_timer > 0.0 {
                continue;
            }
            let team = s.team;
            let mut rng = Rng::for_entity(self.seed ^ 0xBA5E, self.tick, i as u32);
            // Появляемся на базе или на любой своей точке.
            let mut spots = vec![self.bases[team as usize]];
            spots.extend(self.points.iter().filter(|p| p.owner == Some(team)).map(|p| p.pos));
            let spot = spots[rng.below(spots.len() as u32) as usize];
            let pos = self.find_spawn_spot(spot, 6.0, &mut rng);
            let controller = self.soldiers[i].controller;
            self.soldiers[i] = Soldier { controller, ..self.fresh_soldier(team, pos) };
        }
    }

    /// Отдаёт игроку солдата команды, которым пока управляет ИИ.
    /// Живые в приоритете, чтобы игрок сразу оказался в бою.
    pub fn take_control(&mut self, team: u8) -> Option<u32> {
        let free = |s: &&Soldier| s.team == team && s.controller == Controller::Ai;
        let pick = self
            .soldiers
            .iter()
            .enumerate()
            .filter(|(_, s)| free(s))
            .max_by_key(|(_, s)| s.alive)
            .map(|(i, _)| i)?;
        let s = &mut self.soldiers[pick];
        s.controller = Controller::Player(PlayerInput { aim: s.pos, ..Default::default() });
        s.target = None;
        Some(pick as u32)
    }

    /// Игрок ушёл — солдат снова под управлением ИИ.
    pub fn release_control(&mut self, soldier: u32) {
        if let Some(s) = self.soldiers.get_mut(soldier as usize) {
            s.controller = Controller::Ai;
            s.objective = NO_OBJECTIVE;
        }
    }

    pub fn set_input(&mut self, soldier: u32, input: PlayerInput) {
        if let Some(s) = self.soldiers.get_mut(soldier as usize)
            && let Controller::Player(current) = &mut s.controller
        {
            *current = input;
        }
    }

    /// Бросок гранаты в точку (не дальше `max_distance`).
    pub(crate) fn throw_grenade(&mut self, soldier: u32, at: Vec2) {
        let spec = &self.config.grenade;
        let s = &mut self.soldiers[soldier as usize];
        let mut to = at - s.pos;
        let mut distance = to.length();
        if distance < 0.5 {
            return;
        }
        if distance > spec.max_distance {
            to *= spec.max_distance / distance;
            distance = spec.max_distance;
        }
        s.grenades -= 1;
        s.grenade_timer = spec.interval;
        self.projectiles.push(Projectile {
            pos: s.pos,
            vel: to / distance * spec.throw_speed,
            ttl: spec.fuse,
            team: s.team,
            shooter: soldier,
            kind: ProjectileKind::Grenade { fuse: spec.fuse, flight: distance / spec.throw_speed },
        });
        self.events.push(Event::GrenadeThrown { shooter: soldier, from: s.pos, to: s.pos + to, fuse: spec.fuse });
    }

    fn fresh_soldier(&self, team: u8, pos: Vec2) -> Soldier {
        Soldier {
            controller: Controller::Ai,
            pos,
            vel: Vec2::ZERO,
            facing: if team == 0 { 0.0 } else { std::f32::consts::PI },
            team,
            alive: true,
            health: 1.0,
            respawn_timer: 0.0,
            target: None,
            objective: NO_OBJECTIVE,
            cooldown: 0.0,
            ammo: self.config.weapon.magazine_size,
            reload_timer: 0.0,
            grenades: self.config.grenade.count,
            grenade_timer: 0.0,
            kills: 0,
        }
    }

    fn find_spawn_spot(&self, center: Vec2, spread: f32, rng: &mut Rng) -> Vec2 {
        let radius = self.config.soldier.radius;
        for _ in 0..12 {
            let a = rng.range(0.0, std::f32::consts::TAU);
            let r = spread * rng.f32().sqrt();
            let pos = center + Vec2::new(a.cos(), a.sin()) * r;
            if !self.map.circle_hits_solid(pos, radius) {
                return pos;
            }
        }
        center
    }

    pub(crate) fn kill(&mut self, victim: u32, killer: u32) {
        let s = &mut self.soldiers[victim as usize];
        if !s.alive {
            return;
        }
        s.alive = false;
        s.target = None;
        s.respawn_timer = self.config.soldier.respawn_time;
        let victim_team = s.team;
        let k = &mut self.soldiers[killer as usize];
        if k.team != victim_team {
            k.kills += 1;
            self.score[k.team as usize] += 1;
        }
        self.events.push(Event::Kill { killer, victim });
    }
}

/// Позиции и «жив ли» всех солдат на начало фазы: параллельный код читает
/// отсюда, а пишет в самих солдат.
pub(crate) fn snapshot(soldiers: &[Soldier]) -> Vec<(Vec2, bool)> {
    soldiers.iter().map(|s| (s.pos, s.alive)).collect()
}

fn ms(since: Instant) -> f32 {
    since.elapsed().as_secs_f32() * 1000.0
}
