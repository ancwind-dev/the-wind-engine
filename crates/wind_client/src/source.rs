//! Откуда берётся мир: своя симуляция (игра против ИИ) или сервер.
//! Здесь же — превращение мира в списки для отрисовки и данные для HUD.

use std::time::Instant;
use wind_net::replication::{Heard, MAX_HEARD};
use wind_net::{ClientStatus, NetClient};
use wind_render::{CorpseDraw, PointDraw, SoldierDraw};
use wind_sim::arsenal::WeaponSpec;
use wind_sim::round::Round;
use wind_sim::atmosphere::Atmosphere;
use wind_sim::map::Map;
use wind_sim::{Controller, Event, PlayerInput, ProjectileKind, Vec2, World};

/// Что показывает HUD про своего солдата.
#[derive(Clone, Default)]
pub struct Me {
    /// Название оружия.
    pub weapon: String,
    pub alive: bool,
    pub health: f32,
    pub ammo: u32,
    pub magazine: u32,
    pub reloading: bool,
    pub grenades: u32,
    pub respawn_in: f32,
    pub kills: u32,
}

/// Строка ленты убийств.
#[derive(Clone)]
pub struct KillLine {
    pub killer: String,
    pub killer_team: u8,
    pub weapon: String,
    pub victim: String,
    pub victim_team: u8,
    /// Убил свой солдат или убили его — строка выделена.
    pub mine: bool,
}

/// Строка таблицы бойцов.
#[derive(Clone)]
pub struct BoardRow {
    pub name: String,
    pub team: u8,
    pub kills: u32,
    pub deaths: u32,
    pub me: bool,
}

/// Раунд для HUD: идёт ли бой, кто победил, сколько времени, кто держит точки.
#[derive(Clone, Default)]
pub struct RoundView {
    pub round: Round,
    /// Правила: сколько держать все точки, длина раунда, перерыв, с.
    pub hold_time: f32,
    pub time_limit: f32,
    pub intermission: f32,
    pub team_names: [String; 2],
    /// Наша команда (`None` — наблюдаем).
    pub own_team: Option<u8>,
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
    pub kill_feed: Vec<KillLine>,
    /// Таблица бойцов: игроки и лучшие из ИИ, по командам.
    pub board: Vec<BoardRow>,
    pub corpses: Vec<CorpseDraw>,
    /// Есть свой солдат (иначе мы наблюдатель).
    pub has_me: bool,
    pub round: RoundView,
}

/// Оба варианта большие, а `Source` переезжает при смене боя — держим их в куче.
pub enum Source {
    Local(Box<Local>),
    Remote(Box<NetClient>),
}

pub struct Local {
    pub world: World,
    /// Наш солдат. `None` — просто смотрим (замер FPS).
    pub me: Option<u32>,
    accumulator: f32,
    blasts: Vec<(Vec2, f32, f32)>,
    /// Лента убийств: (убийца, убитый, возраст).
    feed: Vec<(u32, u32, f32)>,
    pub sim_ms: f32,
    /// Сколько раз выстрелил свой солдат — для отдачи прицела.
    own_shots: u32,
    /// Что слышно с прошлого кадра.
    heard: Vec<Heard>,
}

impl Local {
    pub fn new(mut world: World, play: bool) -> Self {
        let me = if play { world.take_control(0) } else { None };
        Local { world, me, accumulator: 0.0, blasts: Vec::new(), feed: Vec::new(), sim_ms: 0.0, own_shots: 0, heard: Vec::new() }
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

    /// Номер раунда тоже входит в ключ: в новом раунде следы боя, обломки и руины
    /// прошлого убираются.
    pub fn map_key(&self) -> u64 {
        match self {
            Source::Local(l) => (l.world.seed.wrapping_mul(2) + 1) ^ ((l.world.round.number as u64) << 40),
            Source::Remote(c) => c.world.as_ref().map_or(0, |w| (w.welcome.seed.wrapping_mul(2) + 2) ^ ((w.round.number as u64) << 40)),
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
                            Event::Explosion { pos, radius } => {
                                l.blasts.push((pos, radius, 0.0));
                                l.heard.push(Heard::Blast { pos, radius });
                            }
                            Event::Shot { shooter, .. } => {
                                if Some(shooter) == l.me {
                                    l.own_shots += 1;
                                }
                                let s = &w.soldiers[shooter as usize];
                                l.heard.push(Heard::Shot { pos: s.pos, shooter, team: s.team });
                            }
                            Event::GrenadeThrown { from, .. } => l.heard.push(Heard::Throw { pos: from }),
                            // Только убийства с участием игроков — иначе ленту не прочитать.
                            Event::Kill { killer, victim } => {
                                let player = |i: u32| matches!(w.soldiers[i as usize].controller, Controller::Player(_));
                                if player(killer) || player(victim) {
                                    l.feed.insert(0, (killer, victim, 0.0));
                                }
                            }
                            _ => {}
                        }
                    }
                }
                l.sim_ms = started.elapsed().as_secs_f32() * 1000.0;
                if l.heard.len() > MAX_HEARD {
                    let extra = l.heard.len() - MAX_HEARD;
                    l.heard.drain(..extra);
                }
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

    /// Дальность обзора солдата — для тумана войны.
    pub fn vision_range(&self) -> f32 {
        match self {
            Source::Local(l) => l.world.config.soldier.vision_range,
            Source::Remote(c) => c.world.as_ref().map_or(55.0, |w| w.welcome.vision_range),
        }
    }

    /// Идёт ли бой (а не перерыв между раундами).
    pub fn round_playing(&self) -> bool {
        match self {
            Source::Local(l) => l.world.round.playing(),
            Source::Remote(c) => c.world.as_ref().is_none_or(|w| w.round.playing()),
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

    /// Скорость шага и радиус солдата.
    pub fn soldier_spec(&self) -> (f32, f32) {
        match self {
            Source::Local(l) => (l.world.config.soldier.speed, l.world.config.soldier.radius),
            Source::Remote(c) => c.world.as_ref().map_or((0.0, 0.4), |w| (w.welcome.soldier_speed, w.welcome.soldier_radius)),
        }
    }

    /// Оружие своего солдата.
    pub fn own_weapon(&self) -> Option<WeaponSpec> {
        match self {
            Source::Local(l) => l.me.map(|i| l.world.weapon(i).clone()),
            Source::Remote(c) => c.world.as_ref().map(|w| w.my_weapon().clone()),
        }
    }

    /// Что слышно с прошлого вызова.
    pub fn take_heard(&mut self) -> Vec<Heard> {
        match self {
            Source::Local(l) => std::mem::take(&mut l.heard),
            Source::Remote(c) => c.world.as_mut().map(|w| std::mem::take(&mut w.heard)).unwrap_or_default(),
        }
    }

    /// Оружие солдата `shooter` команды `team`.
    pub fn weapon_of(&self, team: u8, shooter: u32) -> Option<&WeaponSpec> {
        match self {
            Source::Local(l) => (l.world.soldiers.len() > shooter as usize).then(|| l.world.weapon(shooter)),
            Source::Remote(c) => c.world.as_ref().map(|w| w.arsenal.weapon_of(team, shooter)),
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
            .map(|(i, s)| SoldierDraw { id: i as u32, pos: s.drawn_pos(l.accumulator), facing: s.facing, team: s.team, look: None, prone: s.prone })
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
            ProjectileKind::Grenade { .. } | ProjectileKind::Shell { .. } => v.grenades.push(p.pos),
        }
    }
    if let Some(me) = l.me.map(|i| &w.soldiers[i as usize]) {
        v.me_pos = Some(me.drawn_pos(l.accumulator));
        v.has_me = true;
        let weapon = w.weapon(l.me.unwrap_or(0));
        v.me = Me {
            weapon: weapon.name.to_uppercase(),
            alive: me.alive,
            health: me.health,
            ammo: me.ammo,
            magazine: weapon.magazine_size,
            reloading: me.reload_timer > 0.0,
            grenades: me.grenades,
            respawn_in: me.respawn_timer.max(0.0),
            kills: me.kills,
        };
    }
    let name = |i: u32| if Some(i) == l.me { "Ты".to_string() } else { format!("#{i}") };
    v.kill_feed = l
        .feed
        .iter()
        .map(|&(killer, victim, _)| KillLine {
            killer: name(killer),
            killer_team: w.soldiers[killer as usize].team,
            weapon: w.weapon(killer).name.to_uppercase(),
            victim: name(victim),
            victim_team: w.soldiers[victim as usize].team,
            mine: l.me.is_some_and(|m| m == killer || m == victim),
        })
        .collect();
    let players: Vec<(u32, String)> = l.me.map(|m| vec![(m, "Ты".to_string())]).unwrap_or_default();
    v.board = wind_net::replication::board(w, &players)
        .into_iter()
        .map(|e| BoardRow { name: name(e.soldier), team: e.team, kills: e.kills, deaths: e.deaths, me: Some(e.soldier) == l.me })
        .collect();
    let spec = &w.config.round;
    v.round = RoundView {
        round: w.round.clone(),
        hold_time: spec.hold_time,
        time_limit: spec.time_limit,
        intermission: spec.intermission,
        team_names: w.config.arsenal.team_names.clone(),
        own_team: l.me.map(|i| w.soldiers[i as usize].team),
    };
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
    let own = w.welcome.soldier;
    let name = |id: u32| if id == own { "Ты".to_string() } else { w.name_of(id) };
    View {
        soldiers: w
            .soldiers
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                let s = s.as_ref()?;
                Some(SoldierDraw { id: i as u32, pos: w.render_pos(s, now), facing: s.net.facing(), team: s.net.team(), look: None, prone: s.net.is_prone() })
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
            weapon: w.my_weapon().name.to_uppercase(),
            alive: w.me.alive,
            health: w.me.health,
            ammo: w.me.ammo,
            magazine: w.my_weapon().magazine_size,
            reloading: w.me.reloading,
            grenades: w.me.grenades,
            respawn_in: w.me.respawn_in,
            kills: w.me.kills,
        },
        score: w.score,
        kill_feed: w
            .kill_feed
            .iter()
            .map(|k| KillLine {
                killer: name(k.killer),
                killer_team: k.killer_team,
                weapon: w.arsenal.weapon_of(k.killer_team, k.killer).name.to_uppercase(),
                victim: name(k.victim),
                victim_team: w.soldiers.get(k.victim as usize).and_then(|s| s.as_ref()).map_or(1 - (k.killer_team & 1), |s| s.net.team()),
                mine: k.killer == own || k.victim == own,
            })
            .collect(),
        board: w
            .board
            .iter()
            .map(|e| BoardRow {
                name: if e.name.is_empty() { format!("#{}", e.soldier) } else { e.name.clone() },
                team: e.team,
                kills: e.kills,
                deaths: e.deaths,
                me: e.soldier == own,
            })
            .collect(),
        round: RoundView {
            round: w.round.clone(),
            hold_time: w.welcome.hold_time,
            time_limit: w.welcome.time_limit,
            intermission: w.welcome.intermission,
            team_names: w.arsenal.team_names.clone(),
            own_team: Some(w.welcome.team),
        },
        corpses: w
            .corpses
            .iter()
            .map(|c| CorpseDraw { id: c.id, pos: c.pos, facing: c.facing, team: c.team, age: c.age, model: None })
            .collect(),
    }
}
