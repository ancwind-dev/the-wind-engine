//! Откуда берётся мир: своя симуляция (игра против ИИ) или сервер.
//! Здесь же — превращение мира в списки для отрисовки и данные для HUD.

use std::time::Instant;
use wind_net::{ClientStatus, NetClient};
use wind_render::{CorpseDraw, PointDraw, SoldierDraw};
use wind_sim::atmosphere::Atmosphere;
use wind_sim::map::Map;
use wind_sim::{Controller, Event, PlayerInput, ProjectileKind, Vec2, World};

/// Что показывает HUD про своего солдата.
#[derive(Clone, Default)]
pub struct Me {
    pub alive: bool,
    pub health: f32,
    pub ammo: u32,
    pub magazine: u32,
    pub reloading: bool,
    pub grenades: u32,
    pub respawn_in: f32,
    pub kills: u32,
}

/// Кадр мира в простых списках.
#[derive(Default)]
pub struct View {
    pub soldiers: Vec<SoldierDraw>,
    pub bullets: Vec<(Vec2, Vec2)>,
    pub grenades: Vec<Vec2>,
    pub blasts: Vec<(Vec2, f32, f32)>,
    pub points: Vec<PointDraw>,
    pub owners: Vec<Option<u8>>,
    pub me_pos: Option<Vec2>,
    pub me: Me,
    pub score: [u32; 2],
    pub kill_feed: Vec<(String, u8)>,
    pub corpses: Vec<CorpseDraw>,
    /// Есть свой солдат (иначе мы наблюдатель).
    pub has_me: bool,
}

pub enum Source {
    Local(Local),
    Remote(NetClient),
}

pub struct Local {
    pub world: World,
    /// Наш солдат. `None` — просто смотрим (замер FPS).
    pub me: Option<u32>,
    accumulator: f32,
    blasts: Vec<(Vec2, f32, f32)>,
    /// Лента убийств: (текст, команда убийцы, возраст).
    feed: Vec<(String, u8, f32)>,
    pub sim_ms: f32,
    /// Сколько раз выстрелил свой солдат — для отдачи прицела.
    own_shots: u32,
}

impl Local {
    pub fn new(mut world: World, play: bool) -> Self {
        let me = if play { world.take_control(0) } else { None };
        Local { world, me, accumulator: 0.0, blasts: Vec::new(), feed: Vec::new(), sim_ms: 0.0, own_shots: 0 }
    }
}

impl Source {
    pub fn map(&self) -> Option<&Map> {
        match self {
            Source::Local(l) => Some(&l.world.map),
            Source::Remote(c) => c.world.as_ref().map(|w| &w.map),
        }
    }

    /// Ключ карты: при подключении к серверу карта другая.
    pub fn atmosphere(&self) -> Atmosphere {
        match self {
            Source::Local(l) => l.world.atmosphere.clone(),
            Source::Remote(c) => c.world.as_ref().map(|w| w.atmosphere.clone()).unwrap_or_default(),
        }
    }

    pub fn map_key(&self) -> u64 {
        match self {
            Source::Local(l) => l.world.seed.wrapping_mul(2) + 1,
            Source::Remote(c) => c.world.as_ref().map_or(0, |w| w.welcome.seed.wrapping_mul(2) + 2),
        }
    }

    /// Двигает мир на `dt` секунд и отдаёт ввод игрока.
    pub fn update(&mut self, dt: f32, input: PlayerInput) {
        match self {
            Source::Local(l) => {
                if let Some(me) = l.me {
                    l.world.set_input(me, input);
                }
                let step = l.world.dt();
                l.accumulator += dt.min(0.25);
                let started = Instant::now();
                while l.accumulator >= step {
                    l.world.step();
                    l.accumulator -= step;
                    let w = &l.world;
                    for e in &w.events {
                        match *e {
                            Event::Explosion { pos, radius } => l.blasts.push((pos, radius, 0.0)),
                            Event::Shot { shooter, .. } if Some(shooter) == l.me => l.own_shots += 1,
                            // Только убийства с участием игроков — иначе ленту не прочитать.
                            Event::Kill { killer, victim } => {
                                let player = |i: u32| matches!(w.soldiers[i as usize].controller, Controller::Player(_));
                                if player(killer) || player(victim) {
                                    let name = |i: u32| if Some(i) == l.me { "Ты".to_string() } else { format!("#{i}") };
                                    l.feed.insert(0, (format!("{} → {}", name(killer), name(victim)), w.soldiers[killer as usize].team, 0.0));
                                }
                            }
                            _ => {}
                        }
                    }
                }
                l.sim_ms = started.elapsed().as_secs_f32() * 1000.0;
                for b in &mut l.blasts {
                    b.2 += dt;
                }
                l.blasts.retain(|b| b.2 < 0.6);
                for f in &mut l.feed {
                    f.2 += dt;
                }
                l.feed.retain(|f| f.2 < 8.0);
                l.feed.truncate(6);
            }
            Source::Remote(client) => {
                client.poll();
                if let Some(w) = &mut client.world {
                    w.advance_effects(dt);
                }
                client.send_input(input);
            }
        }
    }

    /// Свой солдат по серверу (или по своей симуляции), если он жив.
    pub fn own_server_pos(&self) -> Option<Vec2> {
        match self {
            Source::Local(l) => l.me.map(|i| &l.world.soldiers[i as usize]).filter(|s| s.alive).map(|s| s.pos),
            Source::Remote(c) => {
                let w = c.world.as_ref()?;
                w.my_soldier().filter(|_| w.me.alive).map(|s| s.pos)
            }
        }
    }

    pub fn own_id(&self) -> Option<u32> {
        match self {
            Source::Local(l) => l.me,
            Source::Remote(c) => c.world.as_ref().map(|w| w.welcome.soldier),
        }
    }

    /// Скорость шага и радиус солдата, разброс оружия в градусах.
    pub fn soldier_spec(&self) -> (f32, f32, f32) {
        match self {
            Source::Local(l) => {
                let c = &l.world.config;
                (c.soldier.speed, c.soldier.radius, c.weapon.spread_deg)
            }
            Source::Remote(c) => c
                .world
                .as_ref()
                .map_or((0.0, 0.4, 0.0), |w| (w.welcome.soldier_speed, w.welcome.soldier_radius, w.welcome.spread_deg)),
        }
    }

    /// Сколько раз выстрелил свой солдат.
    pub fn own_shots(&self) -> u32 {
        match self {
            Source::Local(l) => l.own_shots,
            Source::Remote(c) => c.world.as_ref().map_or(0, |w| w.own_shots),
        }
    }

    /// Где свой солдат на экране — оттуда вылетают его трассеры.
    pub fn set_own_pos(&mut self, pos: Option<Vec2>) {
        if let Source::Remote(c) = self
            && let Some(w) = &mut c.world
        {
            w.own_pos = pos;
        }
    }

    pub fn status(&self) -> Option<String> {
        match self {
            Source::Local(_) => None,
            Source::Remote(c) => match &c.status {
                ClientStatus::Connecting => Some(format!("Подключаюсь к {}…", c.server_addr())),
                ClientStatus::Connected => None,
                ClientStatus::Rejected(why) => Some(format!("Сервер отказал: {why}")),
                ClientStatus::TimedOut => Some("Сервер не отвечает".into()),
            },
        }
    }

    pub fn view(&self) -> View {
        match self {
            Source::Local(l) => local_view(l),
            Source::Remote(c) => remote_view(c),
        }
    }
}

fn local_view(l: &Local) -> View {
    let w = &l.world;
    let mut v = View {
        soldiers: w
            .soldiers
            .iter()
            .enumerate()
            .filter(|(_, s)| s.alive)
            .map(|(i, s)| SoldierDraw { id: i as u32, pos: s.drawn_pos(l.accumulator), facing: s.facing, team: s.team, look: None })
            .collect(),
        blasts: l.blasts.clone(),
        points: w
            .points
            .iter()
            .map(|p| PointDraw { pos: p.pos, radius: w.config.capture.radius, owner: p.owner })
            .collect(),
        owners: w.points.iter().map(|p| p.owner).collect(),
        score: w.score,
        ..Default::default()
    };
    for p in &w.projectiles {
        match p.kind {
            ProjectileKind::Bullet => v.bullets.push((p.pos, p.vel)),
            ProjectileKind::Grenade { .. } => v.grenades.push(p.pos),
        }
    }
    if let Some(me) = l.me.map(|i| &w.soldiers[i as usize]) {
        v.me_pos = Some(me.drawn_pos(l.accumulator));
        v.has_me = true;
        v.me = Me {
            alive: me.alive,
            health: me.health,
            ammo: me.ammo,
            magazine: w.config.weapon.magazine_size,
            reloading: me.reload_timer > 0.0,
            grenades: me.grenades,
            respawn_in: me.respawn_timer.max(0.0),
            kills: me.kills,
        };
    }
    v.kill_feed = l.feed.iter().map(|f| (f.0.clone(), f.1)).collect();
    let respawn = w.config.soldier.respawn_time;
    v.corpses = w
        .soldiers
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.alive && s.respawn_timer > 0.0)
        .map(|(i, s)| CorpseDraw { id: i as u32, pos: s.pos, facing: s.facing, team: s.team, age: respawn - s.respawn_timer, model: None })
        .collect();
    v
}

fn remote_view(c: &NetClient) -> View {
    let Some(w) = &c.world else { return View::default() };
    let now = c.now();
    let name = |id: u32| if id == w.welcome.soldier { "Ты".to_string() } else { format!("#{id}") };
    View {
        soldiers: w
            .soldiers
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                let s = s.as_ref()?;
                Some(SoldierDraw { id: i as u32, pos: w.render_pos(s, now), facing: s.net.facing(), team: s.net.team(), look: None })
            })
            .collect(),
        bullets: w.tracers.iter().map(|t| (t.pos, t.vel)).collect(),
        grenades: w.grenades.iter().map(|g| g.pos).collect(),
        blasts: w.blasts.clone(),
        points: w
            .points
            .iter()
            .map(|p| PointDraw { pos: p.pos, radius: w.welcome.capture_radius, owner: p.owner })
            .collect(),
        owners: w.points.iter().map(|p| p.owner).collect(),
        me_pos: w.my_soldier().map(|s| w.render_pos(s, now)),
        has_me: true,
        me: Me {
            alive: w.me.alive,
            health: w.me.health,
            ammo: w.me.ammo,
            magazine: w.welcome.magazine_size,
            reloading: w.me.reloading,
            grenades: w.me.grenades,
            respawn_in: w.me.respawn_in,
            kills: w.me.kills,
        },
        score: w.score,
        kill_feed: w.kill_feed.iter().map(|k| (format!("{} → {}", name(k.killer), name(k.victim)), k.killer_team)).collect(),
        corpses: w
            .corpses
            .iter()
            .map(|c| CorpseDraw { id: c.id, pos: c.pos, facing: c.facing, team: c.team, age: c.age, model: None })
            .collect(),
    }
}
