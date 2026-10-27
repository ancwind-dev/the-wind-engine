//! Мир и тик симуляции.

use crate::arsenal::{BlastSpec, WeaponSpec};
use crate::command::{self, PointPlan, SquadInfo};
use crate::config::SimConfig;
use crate::cover::Cover;
use crate::grid::SpatialGrid;
use crate::map::Map;
use crate::atmosphere::Atmosphere;
use crate::mapfile::MapFile;
use crate::mapgen::{self, Layout};
use crate::nav::FlowField;
use crate::rng::Rng;
use crate::round::{EndReason, Phase, Round};
use crate::squad::{self, NO_SQUAD, Order, Squad};
use crate::suppression::RETREAT_TO_BASE;
use crate::{ai, combat};
use glam::Vec2;
use rayon::prelude::*;
use std::time::Instant;

/// Солдат не идёт ни к какой точке (выберет на следующем «обдумывании»).
pub const NO_OBJECTIVE: u32 = u32::MAX;

/// Устаревшее поле путей пересчитываем не чаще, тиков (15 с).
const NAV_REFRESH: u64 = 300;

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
    /// Где появиться после смерти: 0 — на базе, 1 + i — на точке i, если она наша
    /// (иначе на базе).
    pub spawn: u8,
    /// Где своего солдата видит клиент. Клиент двигает его сам, без задержки на пинг,
    /// а сервер подтягивает солдата туда, но не быстрее `CLAIM_CATCH_UP` скоростей
    /// и не сквозь стены. `None` — двигать по `move_dir`, как раньше (боты, лаборатория).
    pub pos: Option<Vec2>,
}

/// Во сколько раз быстрее обычного шага сервер может догонять позицию клиента:
/// пакеты с вводом приходят неровно, и солдат должен успевать.
pub const CLAIM_CATCH_UP: f32 = 1.5;
/// Если клиент дальше этого от сервера (возрождение, застрял, подделка), его позицию
/// не слушаем — клиент сам вернёт солдата на место сервера.
pub const CLAIM_MAX_GAP: f32 = 2.0;

/// Шаг солдата со скольжением вдоль стен: по осям отдельно. Один и тот же на сервере
/// и на клиенте — иначе клиент и сервер разойдутся у стен.
pub fn walk(map: &Map, pos: Vec2, step: Vec2, radius: f32) -> Vec2 {
    let mut pos = pos;
    let nx = pos + Vec2::new(step.x, 0.0);
    if !map.circle_hits_solid(nx, radius) {
        pos = nx;
    }
    let ny = pos + Vec2::new(0.0, step.y);
    if !map.circle_hits_solid(ny, radius) {
        pos = ny;
    }
    pos
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
    /// Убийства и смерти за раунд — для таблицы. При возрождении не сбрасываются.
    pub kills: u32,
    pub deaths: u32,
    /// Набор (класс): номер в `arsenal.teams[team]`. Выбирается по номеру солдата.
    pub kit: u8,
    /// «Нагрев» очереди: разброс растёт с каждым выстрелом и спадает со временем.
    pub heat: f32,
    /// Номер отряда в `World::squads` (`NO_SQUAD` — без отряда). Не меняется весь матч.
    pub squad: u32,
    /// Укрытие (угол, из-за которого стреляет) и сколько ещё за него держаться, с.
    pub cover: Option<Cover>,
    pub cover_timer: f32,
    /// Через сколько секунд можно снова искать укрытие.
    pub cover_search: f32,
    /// Откуда по солдату стреляли (пуля прошла рядом или попала) и сколько ещё
    /// он это помнит, с.
    pub threat: Option<Vec2>,
    pub under_fire: f32,
    /// Подавление огнём, 0..1: пули рядом, спадает само (см. `suppression`).
    pub suppression: f32,
    /// Прижат огнём (подавление было выше `prone_at` и ещё не спало до `stand_at`).
    pub pinned: bool,
    /// Залёг: прижат огнём в чистом поле — почти не двигается, попасть труднее.
    /// В укрытии прижатый не ложится, а уходит за угол.
    pub prone: bool,
    /// Где последний раз видел врага и сколько ещё туда стрелять на подавление, с.
    pub last_seen: Option<Vec2>,
    pub last_seen_timer: f32,
    /// Сколько секунд нет боя (для заживления ран).
    pub calm: f32,
    /// Ранен и отходит: к точке захвата с этим номером или на базу (`RETREAT_TO_BASE`).
    pub retreat: Option<u32>,
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
    /// Снаряд гранатомёта: летит `flight` секунд к точке прицела и взрывается —
    /// там или раньше, если задел стену или врага.
    Shell { flight: f32 },
}

#[derive(Clone, Debug)]
pub struct Projectile {
    pub pos: Vec2,
    pub vel: Vec2,
    pub ttl: f32,
    pub team: u8,
    pub shooter: u32,
    /// Из какого оружия (номер в `arsenal.weapons`): от него урон и взрыв.
    pub weapon: u8,
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
    /// Раунд кончился: победитель (`None` — ничья).
    RoundOver { winner: Option<u8> },
    /// Начался новый раунд: карта, точки и счёт — как в начале.
    RoundStarted { number: u32 },
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
    /// Поле путей к базе каждой команды — для отхода раненых.
    pub base_flow: Vec<FlowField>,
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
    /// Раунд: идёт бой или перерыв, сколько времени, кто держит все точки.
    pub round: Round,
    /// Отряды обеих команд и приказ каждому солдату на этот тик (по номеру солдата).
    pub squads: Vec<Squad>,
    pub orders: Vec<Order>,
    /// Есть ли штаб у команды (из настроек; в тестах можно выключить одной команде)
    /// и как он видит точки на последнем совещании.
    pub command_enabled: [bool; 2],
    pub plans: [Vec<PointPlan>; 2],
    /// Карта в начале — новый раунд начинается с целыми стенами.
    initial_map: Map,
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
        let mut base_flow: Vec<FlowField> = bases.iter().map(|&p| FlowField::new(p)).collect();
        base_flow.par_iter_mut().for_each(|f| f.compute(&map));

        let points = capture_points
            .iter()
            .map(|&pos| CapturePoint { pos, owner: None, progress: 0.0 })
            .collect();
        let grid = SpatialGrid::new(map.width.max(map.height) as f32, GRID_CELL);
        let command_on = config.command.enabled;

        let mut world = World {
            config,
            seed,
            tick: 0,
            map_file: None,
            atmosphere: Atmosphere::default(),
            round: Round::default(),
            squads: Vec::new(),
            orders: Vec::new(),
            command_enabled: [command_on; 2],
            plans: [Vec::new(), Vec::new()],
            initial_map: map.clone(),
            map,
            bases,
            points,
            flow,
            base_flow,
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
                let kit = world.config.arsenal.pick(team, world.soldiers.len() as u32) as u8;
                let mut soldier = world.fresh_soldier(team, pos, kit);
                // Чтобы не стреляли все в один тик.
                soldier.cooldown = rng.range(0.0, 1.0);
                world.soldiers.push(soldier);
            }
        }
        world.squads = squad::form(&mut world.soldiers, world.config.squad.size);
        world
    }

    pub fn dt(&self) -> f32 {
        self.config.dt()
    }

    pub fn step(&mut self) {
        let start = Instant::now();
        self.events.clear();
        let mut stats = TickStats::default();

        // Перерыв между раундами: бой стоит, ждём начала нового.
        if !self.round.playing() {
            self.round.time += self.dt();
            if self.round.time >= self.config.round.intermission {
                self.new_round();
            }
            self.tick += 1;
            stats.total_ms = ms(start);
            self.stats = stats;
            return;
        }

        let t = Instant::now();
        self.rebuild_grid();
        stats.grid_ms = ms(t);

        let t = Instant::now();
        self.update_navigation();
        stats.nav_ms = ms(t);

        // Строй отрядов и решения ИИ — в одной фазе замера.
        let t = Instant::now();
        let every = (self.config.command.interval * self.config.tick_rate as f32).round().max(1.0) as u64;
        if self.tick.is_multiple_of(every) {
            self.run_command();
        }
        self.plan_squads();
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
        self.update_round();
        stats.capture_ms = ms(t);

        for p in &self.projectiles {
            match p.kind {
                ProjectileKind::Bullet => stats.bullets += 1,
                ProjectileKind::Grenade { .. } | ProjectileKind::Shell { .. } => stats.grenades += 1,
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

    /// Штаб каждой команды раздаёт отрядам точки (см. `command`).
    fn run_command(&mut self) {
        let r = self.config.command.threat_radius;
        for team in 0..2u8 {
            let t = team as usize;
            if !self.command_enabled[t] {
                self.plans[t].clear();
                continue;
            }
            // Сколько своих и врагов у каждой точки.
            let (mut ours, mut enemies) = (vec![0u32; self.points.len()], vec![0u32; self.points.len()]);
            for (i, p) in self.points.iter().enumerate() {
                let soldiers = &self.soldiers;
                self.grid.query(p.pos, r, |j| {
                    let s = &soldiers[j as usize];
                    if s.alive && s.pos.distance_squared(p.pos) < r * r {
                        if s.team == team { ours[i] += 1 } else { enemies[i] += 1 }
                    }
                });
            }
            let ready = |m: &u32| {
                let s = &self.soldiers[*m as usize];
                s.alive && s.controller == Controller::Ai && s.retreat.is_none()
            };
            let mine: Vec<usize> = (0..self.squads.len()).filter(|&q| self.squads[q].team == team && self.squads[q].leader.is_some()).collect();
            let infos: Vec<SquadInfo> = mine
                .iter()
                .map(|&q| {
                    let squad = &self.squads[q];
                    SquadInfo {
                        pos: self.soldiers[squad.leader.unwrap_or(0) as usize].pos,
                        strength: squad.members.iter().filter(|m| ready(m)).count() as u32,
                        current: squad.objective,
                    }
                })
                .collect();
            let (orders, plans) = command::plan(&self.points, team, self.bases[t], &infos, &ours, &enemies, &self.config.command);
            for (&q, &objective) in mine.iter().zip(&orders) {
                self.squads[q].objective = objective;
                for k in 0..self.squads[q].members.len() {
                    let m = self.squads[q].members[k] as usize;
                    self.soldiers[m].objective = objective;
                }
            }
            self.plans[t] = plans;
        }
    }

    fn plan_squads(&mut self) {
        let points: Vec<Vec2> = self.points.iter().map(|p| p.pos).collect();
        squad::plan(
            &mut self.squads,
            &self.soldiers,
            &mut self.orders,
            &self.map,
            &self.config.squad,
            &points,
            self.config.capture.radius,
            self.config.soldier.radius,
        );
    }

    fn rebuild_grid(&mut self) {
        let soldiers = &self.soldiers;
        self.grid.rebuild(soldiers.len(), |i| soldiers[i].alive.then_some(soldiers[i].pos));
    }

    /// После разрушений поля путей устаревают. Пересчитываем не больше одного
    /// поля за тик, чтобы взрыв не давал рывка.
    /// Поля путей после разрушений устаревают. Но в бою стены только рушатся,
    /// новых не бывает — устаревшее поле по-прежнему ведёт куда надо, просто не
    /// знает о новых проломах. Поэтому пересчитываем не больше одного поля за тик
    /// и каждое — не чаще раза в `NAV_REFRESH` тиков: в застроенном городе стены
    /// рушатся почти каждый тик, а поле на карте 1024 считается миллисекунды.
    fn update_navigation(&mut self) {
        let map = &self.map;
        let tick = self.tick;
        let due = |f: &FlowField| f.is_stale(map) && tick >= f.computed_tick + NAV_REFRESH;
        if let Some(field) = self.flow.iter_mut().chain(self.base_flow.iter_mut()).filter(|f| due(f)).min_by_key(|f| f.computed_tick) {
            field.compute(map);
            field.computed_tick = tick;
        }
    }

    /// Все поля сразу — когда стены вернулись на место (новый раунд): старые поля
    /// могут вести сквозь восстановленные стены.
    fn rebuild_navigation(&mut self) {
        let map = &self.map;
        let tick = self.tick;
        self.flow.par_iter_mut().chain(self.base_flow.par_iter_mut()).for_each(|f| {
            f.compute(map);
            f.computed_tick = tick;
        });
    }

    fn move_soldiers(&mut self) {
        let dt = self.dt();
        let spec = &self.config.soldier;
        let capture_radius = self.config.capture.radius;
        let (seed, tick) = (self.seed, self.tick);
        let prev = snapshot(&self.soldiers);
        let map = &self.map;
        let flow = &self.flow;
        let grid = &self.grid;
        let radius = spec.radius;
        let arsenal = &self.config.arsenal;
        let orders = &self.orders;
        let squad_spec = &self.config.squad;
        let sup = &self.config.suppression;
        let base_flow = &self.base_flow;

        self.soldiers.par_iter_mut().enumerate().for_each(|(i, s)| {
            if !s.alive {
                return;
            }
            let mut desired = Vec2::ZERO;
            let mut speed = spec.speed;
            let order = orders.get(i).copied().unwrap_or_default();
            // Боец, далеко ушедший от своего места в строю, в бой не ввязывается:
            // догоняет отряд (стреляет он и на ходу).
            // Раненый, отходящий в тыл, в бой тоже не ввязывается.
            let fight = s.retreat.is_none()
                && match order {
                    Order::Follow { slot, .. } => slot.distance(s.pos) < squad_spec.leash,
                    _ => true,
                };
            // Подавление спадает; сильно подавленного солдата ИИ прижимает к земле, пока
            // не отпустит: в поле он залегает, в укрытии — прячется за угол. Вне боя
            // раны понемногу заживают.
            let ai = s.controller == Controller::Ai;
            s.suppression = (s.suppression - sup.decay * dt).max(0.0);
            s.pinned = ai && if s.pinned { s.suppression > sup.stand_at } else { s.suppression >= sup.prone_at };
            s.prone = s.pinned && (s.cover.is_none() || !fight);
            if s.prone {
                speed *= sup.crawl;
            }
            s.last_seen_timer -= dt;
            s.calm = if s.under_fire > 0.0 || s.target.is_some() { 0.0 } else { s.calm + dt };
            if s.calm >= sup.heal_delay {
                s.health = (s.health + sup.heal_rate * dt).min(1.0);
            }
            if !ai || s.health >= sup.return_above {
                s.retreat = None;
            }
            s.under_fire = (s.under_fire - dt).max(0.0);
            s.cover_search -= dt;
            s.cover_timer -= dt;
            if s.cover_timer <= 0.0 {
                s.cover = None;
            }

            if let Controller::Player(input) = s.controller {
                let look = input.aim - s.pos;
                if look != Vec2::ZERO {
                    s.facing = look.y.atan2(look.x);
                }
                if let Some(claim) = input.pos.filter(|c| c.distance(s.pos) < CLAIM_MAX_GAP) {
                    // Клиент ведёт своего солдата сам — догоняем его заявку.
                    let step = (claim - s.pos).clamp_length_max(spec.speed * CLAIM_CATCH_UP * dt);
                    s.vel = step / dt;
                    s.pos = walk(map, s.pos, step, radius);
                    return;
                }
                desired = input.move_dir.clamp_length_max(1.0);
            } else if let Some(to) = s.retreat {
                // Раненый отходит по полю путей к своей точке или на базу и ждёт там.
                let field = if to == RETREAT_TO_BASE { base_flow.get(s.team as usize & 1) } else { flow.get(to as usize) };
                if let Some(field) = field {
                    let off = field.target - s.pos;
                    if off.length() > capture_radius * 0.5 {
                        desired = field.direction(map, s.pos).unwrap_or_else(|| off.normalize_or_zero());
                        s.facing = desired.y.atan2(desired.x);
                    }
                }
            } else if let Some(c) = s.cover.filter(|_| fight) {
                // Укрытие: стреляем из-за угла, перезаряжаемся и пережидаем огонь за ним.
                let spot = if s.reload_timer > 0.0 || s.pinned { c.hide } else { c.shoot };
                let to = spot - s.pos;
                let d = to.length();
                if d > 0.1 {
                    desired = to / d * d.min(1.0);
                }
                let look = c.threat - s.pos;
                if look != Vec2::ZERO {
                    s.facing = look.y.atan2(look.x);
                }
            } else if let Some(target) = s.target.filter(|&t| fight && prev[t as usize].1) {
                // Враг виден: подходим на дистанцию огня и стоим.
                let to = prev[target as usize].0 - s.pos;
                let d = to.length();
                s.facing = to.y.atan2(to.x);
                // Подходим на дальность своего оружия: у пистолета-пулемёта она короче.
                let range = arsenal.weapons[arsenal.teams[s.team as usize & 1][s.kit as usize].weapon].range;
                if d > range * 0.8 {
                    desired = to / d;
                    speed *= 0.7;
                }
            } else if let Order::Follow { slot, leader_vel, heading } = order
                && slot.distance(s.pos) < squad_spec.rejoin
                && map.line_of_sight(s.pos, slot)
            {
                // Место в строю: идём вместе с командиром и подтягиваемся к своему месту.
                let v = (leader_vel + (slot - s.pos) * 1.5).clamp_length_max(spec.speed);
                desired = v / spec.speed;
                let look = if v.length_squared() > 0.5 { v } else { heading };
                if look != Vec2::ZERO {
                    s.facing = look.y.atan2(look.x);
                }
            } else if let Some(field) = flow.get(s.objective as usize) {
                if let Order::Lead { pace } = order {
                    speed *= pace;
                }
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
            s.pos = walk(map, s.pos, s.vel * dt, radius);
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

    /// Кто держит все точки и не вышло ли время.
    fn update_round(&mut self) {
        let dt = self.dt();
        let spec = &self.config.round;
        let round = &mut self.round;
        round.time += dt;
        let mut owned = [0u32; 2];
        for p in &self.points {
            if let Some(t) = p.owner {
                owned[t as usize] += 1;
            }
        }
        let all = self.points.len() as u32;
        for (hold, &owned) in round.hold.iter_mut().zip(&owned) {
            *hold = if all > 0 && owned == all { *hold + dt } else { 0.0 };
        }
        let end = if let Some(team) = (0..2).find(|&t| round.hold[t] >= spec.hold_time) {
            Some((Some(team as u8), EndReason::AllPoints))
        } else if spec.time_limit > 0.0 && round.time >= spec.time_limit {
            Some((Round::time_up_winner(owned), EndReason::TimeUp))
        } else {
            None
        };
        if let Some((winner, reason)) = end {
            round.phase = Phase::Over { winner, reason };
            round.time = 0.0;
            self.projectiles.clear();
            self.events.push(Event::RoundOver { winner });
        }
    }

    /// Новый раунд на той же карте: стены, точки и счёт — как в начале,
    /// все живы на своих базах.
    fn new_round(&mut self) {
        self.round = Round { number: self.round.number + 1, ..Round::default() };
        // Восстанавливаем только изменившиеся чанки — клиенты получат их, как обычно.
        for chunk in 0..self.map.chunk_revisions.len() {
            let cells = self.initial_map.chunk_cells(chunk);
            if cells != self.map.chunk_cells(chunk) {
                let revision = self.map.chunk_revisions[chunk] + 1;
                self.map.set_chunk_cells(chunk, &cells, revision);
            }
        }
        self.rebuild_navigation();
        for p in &mut self.points {
            p.owner = None;
            p.progress = 0.0;
        }
        self.projectiles.clear();
        self.score = [0; 2];
        for squad in &mut self.squads {
            squad.leader = None;
            squad.objective = NO_OBJECTIVE;
            squad.heading = Vec2::ZERO;
        }
        self.plans = [Vec::new(), Vec::new()];
        let mut rng = Rng::for_entity(self.seed ^ 0x4E57, self.tick, 0);
        let per_team = self.soldiers.iter().filter(|s| s.team == 0).count().max(1) as f32;
        for i in 0..self.soldiers.len() {
            let s = &self.soldiers[i];
            let (team, controller, kit) = (s.team, s.controller, s.kit);
            let pos = self.find_spawn_spot(self.bases[team as usize], per_team.sqrt() * 0.8, &mut rng);
            let mut fresh = Soldier { controller, squad: s.squad, ..self.fresh_soldier(team, pos, kit) };
            fresh.cooldown = rng.range(0.0, 1.0);
            self.soldiers[i] = fresh;
        }
        self.events.push(Event::RoundStarted { number: self.round.number });
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
            let spot = match self.soldiers[i].controller {
                // Игрок выбирает сам: база или своя точка (чужая — значит, база).
                Controller::Player(input) => match input.spawn.checked_sub(1).and_then(|p| self.points.get(p as usize)) {
                    Some(p) if p.owner == Some(team) => p.pos,
                    _ => self.bases[team as usize],
                },
                // ИИ — на базе или на своей точке: ближе всего к командиру отряда.
                Controller::Ai => {
                    let mut spots = vec![self.bases[team as usize]];
                    spots.extend(self.points.iter().filter(|p| p.owner == Some(team)).map(|p| p.pos));
                    let leader = self
                        .squads
                        .get(self.soldiers[i].squad as usize)
                        .and_then(|q| q.leader)
                        .map(|l| &self.soldiers[l as usize])
                        .filter(|l| l.alive);
                    // Без командира — ближе к точке, куда штаб послал отряд: туда и
                    // идут подкрепления.
                    let goal = leader.map(|l| l.pos).or_else(|| {
                        let squad = self.squads.get(self.soldiers[i].squad as usize)?;
                        self.points.get(squad.objective as usize).map(|p| p.pos)
                    });
                    match goal {
                        Some(g) => *spots.iter().min_by(|a, b| a.distance_squared(g).total_cmp(&b.distance_squared(g))).unwrap(),
                        None => spots[rng.below(spots.len() as u32) as usize],
                    }
                }
            };
            let pos = self.find_spawn_spot(spot, 6.0, &mut rng);
            let s = &self.soldiers[i];
            let (controller, kit, kills, deaths, squad) = (s.controller, s.kit, s.kills, s.deaths, s.squad);
            self.soldiers[i] = Soldier { controller, kills, deaths, squad, ..self.fresh_soldier(team, pos, kit) };
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
            weapon: 0,
            kind: ProjectileKind::Grenade { fuse: spec.fuse, flight: distance / spec.throw_speed },
        });
        self.events.push(Event::GrenadeThrown { shooter: soldier, from: s.pos, to: s.pos + to, fuse: spec.fuse });
    }

    /// Живые солдаты около `center` — по сетке, построенной в начале тика. Сетка
    /// грубая: `f` зовётся и для чуть более дальних, расстояние проверяй сам.
    pub fn soldiers_near(&self, center: Vec2, radius: f32, f: impl FnMut(u32)) {
        self.grid.query(center, radius, f);
    }

    /// Оружие солдата.
    pub fn weapon(&self, soldier: u32) -> &WeaponSpec {
        let s = &self.soldiers[soldier as usize];
        let arsenal = &self.config.arsenal;
        &arsenal.weapons[arsenal.teams[s.team as usize & 1][s.kit as usize].weapon]
    }

    /// Взрыв ручной гранаты — в тех же величинах, что и у гранатомёта.
    pub(crate) fn grenade_blast(&self) -> BlastSpec {
        let g = &self.config.grenade;
        BlastSpec {
            radius: g.blast_radius,
            kill_probability: g.kill_probability,
            wall_damage: g.wall_damage,
            wall_damage_radius: g.wall_damage_radius,
        }
    }

    fn fresh_soldier(&self, team: u8, pos: Vec2, kit: u8) -> Soldier {
        let arsenal = &self.config.arsenal;
        let weapon = &arsenal.weapons[arsenal.teams[team as usize & 1][kit as usize].weapon];
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
            ammo: weapon.magazine_size,
            reload_timer: 0.0,
            grenades: self.config.grenade.count,
            grenade_timer: 0.0,
            kills: 0,
            deaths: 0,
            kit,
            heat: 0.0,
            squad: NO_SQUAD,
            cover: None,
            cover_timer: 0.0,
            cover_search: 0.0,
            threat: None,
            under_fire: 0.0,
            suppression: 0.0,
            pinned: false,
            prone: false,
            last_seen: None,
            last_seen_timer: 0.0,
            calm: 0.0,
            retreat: None,
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
        s.deaths += 1;
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
