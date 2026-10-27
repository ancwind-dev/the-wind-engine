//! Репликация: как сервер собирает снимок для клиента и как клиент его разбирает.
//!
//! Снимок — это разница с последним снимком, который клиент подтвердил.
//! Потерянные пакеты не переотправляются: всё, что не дошло, остаётся «не таким,
//! как у клиента» и уедет в следующем снимке. Подробно — в заметке
//! «Сеть — снимки и дельты».
//!
//! Здесь нет сокетов: на вход и на выход — байты. Поэтому всё это легко тестировать.

use crate::bits::{BitReader, BitWriter, Truncated};
use crate::protocol::{self, Kind, Welcome, dq_angle, q_angle, read_pos, write_pos};
use std::collections::VecDeque;
use wind_sim::map::Map;
use std::path::Path;
use wind_sim::arsenal::{Arsenal, WeaponSpec};
use wind_sim::atmosphere::Atmosphere;
use wind_sim::round::{EndReason, Phase, Round};
use wind_sim::mapfile::MapFile;
use wind_sim::{Event, Soldier, Vec2, World, mapgen};

/// Сколько отправленных снимков помнит сервер и сколько полученных — клиент.
pub const HISTORY: usize = 32;

const FLAG_TEAM1: u8 = 1;
const FLAG_PLAYER: u8 = 2;
const FLAG_RELOADING: u8 = 4;

/// Солдат в том виде, в каком он уходит по сети.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetSoldier {
    pub x: u16,
    pub y: u16,
    pub facing: u8,
    pub flags: u8,
}

impl NetSoldier {
    pub fn from_soldier(s: &Soldier) -> Self {
        let (x, y) = protocol::q_vec(s.pos);
        let mut flags = 0;
        if s.team == 1 {
            flags |= FLAG_TEAM1;
        }
        if matches!(s.controller, wind_sim::Controller::Player(_)) {
            flags |= FLAG_PLAYER;
        }
        if s.reload_timer > 0.0 {
            flags |= FLAG_RELOADING;
        }
        NetSoldier { x, y, facing: q_angle(s.facing, 8) as u8, flags }
    }

    pub fn pos(&self) -> Vec2 {
        protocol::dq_vec((self.x, self.y))
    }

    pub fn facing(&self) -> f32 {
        dq_angle(self.facing as u32, 8)
    }

    pub fn team(&self) -> u8 {
        self.flags & FLAG_TEAM1
    }

    pub fn is_player(&self) -> bool {
        self.flags & FLAG_PLAYER != 0
    }
}

/// Что видит клиент: солдаты по возрастанию id.
type View = Vec<(u32, NetSoldier)>;

#[derive(Clone, Debug)]
pub struct ReplicationConfig {
    /// Радиус вокруг своего солдата, в котором клиент видит других, м.
    pub interest_radius: f32,
    /// Желательный размер пакета, байт. Больше — только если не влез один чанк карты.
    pub packet_budget: usize,
    /// Через сколько тиков переслать чанк карты, если клиент его так и не подтвердил.
    pub chunk_resend_ticks: u32,
}

impl Default for ReplicationConfig {
    fn default() -> Self {
        ReplicationConfig { interest_radius: 120.0, packet_budget: 1200, chunk_resend_ticks: 10 }
    }
}

struct Sent {
    tick: u32,
    view: View,
    chunks: Vec<(u32, u32)>,
}

// =========================================================================
// Сервер
// =========================================================================

/// Всё, что сервер помнит про одного клиента.
pub struct ServerReplica {
    pub soldier: u32,
    history: Vec<Option<Sent>>,
    acked: Option<u32>,
    chunk_acked: Vec<u32>,
    /// (версия, тик) последней отправки каждого чанка.
    chunk_sent: Vec<(u32, u32)>,
    /// Накопленный приоритет каждого солдата: кого давно не обновляли, тот растёт,
    /// поэтому дальние солдаты не «голодают», даже когда пакет переполнен.
    priority: Vec<f32>,
    /// Время клиента из последнего ввода — вернём его для подсчёта пинга.
    pub echo_time: u32,
}

impl ServerReplica {
    pub fn new(soldier: u32, world: &World) -> Self {
        let chunks = world.map.chunk_revisions.len();
        ServerReplica {
            soldier,
            history: (0..HISTORY).map(|_| None).collect(),
            acked: None,
            chunk_acked: vec![0; chunks],
            chunk_sent: vec![(0, 0); chunks],
            priority: vec![0.0; world.soldiers.len()],
            echo_time: 0,
        }
    }

    /// Клиент сообщил, что получил снимок тика `tick`.
    pub fn on_ack(&mut self, tick: u32) {
        if tick == 0 || self.acked.is_some_and(|a| a >= tick) {
            return;
        }
        let Some(sent) = &self.history[tick as usize % HISTORY] else { return };
        if sent.tick != tick {
            return;
        }
        self.acked = Some(tick);
        for &(chunk, rev) in &sent.chunks {
            let acked = &mut self.chunk_acked[chunk as usize];
            *acked = (*acked).max(rev);
        }
    }

    /// Снимок для клиента. `board` — таблица бойцов, если пора её разослать
    /// (собирается одна на всех, см. [`board`]).
    pub fn build_snapshot(&mut self, world: &World, cfg: &ReplicationConfig, board: Option<&BitWriter>) -> Vec<u8> {
        let tick = world.tick as u32;
        let me = &world.soldiers[self.soldier as usize];
        let center = me.pos;
        let budget_bits = cfg.packet_budget * 8;

        let baseline = self.acked.and_then(|t| {
            self.history[t as usize % HISTORY]
                .as_ref()
                .filter(|s| s.tick == t && tick.wrapping_sub(t) < HISTORY as u32)
        });
        let empty = View::new();
        let base_view = baseline.map_or(&empty, |s| &s.view);

        // Кого клиент должен видеть сейчас.
        let r2 = cfg.interest_radius * cfg.interest_radius;
        let current: View = world
            .soldiers
            .iter()
            .enumerate()
            .filter(|(_, s)| s.alive && s.pos.distance_squared(center) < r2)
            .map(|(i, s)| (i as u32, NetSoldier::from_soldier(s)))
            .collect();

        // Сравниваем с тем, что у клиента уже есть.
        let mut removed = Vec::new();
        let mut changed: Vec<(u32, NetSoldier, Option<NetSoldier>)> = Vec::new();
        let (mut a, mut b) = (0, 0);
        while a < base_view.len() || b < current.len() {
            match (base_view.get(a), current.get(b)) {
                (Some(&(bid, bs)), Some(&(cid, cs))) if bid == cid => {
                    if bs != cs {
                        changed.push((cid, cs, Some(bs)));
                    }
                    a += 1;
                    b += 1;
                }
                (Some(&(bid, _)), Some(&(cid, _))) if bid < cid => {
                    removed.push(bid);
                    a += 1;
                }
                (Some(&(bid, _)), None) => {
                    removed.push(bid);
                    a += 1;
                }
                (_, Some(&(cid, cs))) => {
                    changed.push((cid, cs, None));
                    b += 1;
                }
                (None, None) => unreachable!(),
            }
        }

        // Заголовок, свой солдат, счёт, точки.
        let mut w = protocol::begin(Kind::Snapshot);
        w.write_bits(tick, 32);
        w.write_var(baseline.map_or(0, |s| tick - s.tick));
        w.write_bits(self.echo_time, 32);
        write_self(&mut w, me);
        w.write_var(world.score[0]);
        w.write_var(world.score[1]);
        w.write_var(world.points.len() as u32);
        for p in &world.points {
            w.write_bits(p.owner.map_or(0, |t| t as u32 + 1), 2);
            w.write_signed((p.progress * 127.0).round() as i32, 8);
        }
        write_round(&mut w, &world.round);
        w.write_bool(board.is_some());
        if let Some(b) = board {
            w.append(b);
        }

        let mut removed_w = BitWriter::new();
        removed_w.write_var(removed.len() as u32);
        let mut prev = None;
        for &id in &removed {
            write_gap(&mut removed_w, &mut prev, id);
        }

        // Самые «заждавшиеся» солдаты — первыми.
        for (id, s, _) in &changed {
            let d = s.pos().distance(center);
            let weight = if *id == self.soldier {
                1000.0
            } else if s.is_player() {
                2.0
            } else {
                1.0
            };
            self.priority[*id as usize] += weight / (1.0 + d / 16.0);
        }
        changed.sort_by(|x, y| self.priority[y.0 as usize].total_cmp(&self.priority[x.0 as usize]));

        let reserve = 64;
        let mut left = budget_bits as i64 - (w.len_bits() + removed_w.len_bits() + reserve) as i64;
        let mut selected = Vec::new();
        for (id, s, base) in changed {
            let mut scratch = BitWriter::new();
            encode_entity(&mut scratch, s, base);
            let cost = scratch.len_bits() as i64 + 16; // + примерно id
            if cost > left {
                continue;
            }
            left -= cost;
            self.priority[id as usize] = 0.0;
            selected.push((id, s, base));
        }
        selected.sort_by_key(|e| e.0);

        let mut entities_w = BitWriter::new();
        entities_w.write_var(selected.len() as u32);
        let mut prev = None;
        for &(id, s, base) in &selected {
            write_gap(&mut entities_w, &mut prev, id);
            encode_entity(&mut entities_w, s, base);
        }

        // События рядом с клиентом.
        let mut events_w = BitWriter::new();
        let mut event_count = 0;
        let near = |p: Vec2, extra: f32| {
            let r = cfg.interest_radius + extra;
            p.distance_squared(center) < r * r
        };
        for event in &world.events {
            let mut scratch = BitWriter::new();
            match *event {
                Event::Shot { shooter, angle } => {
                    if !near(world.soldiers[shooter as usize].pos, 60.0) {
                        continue;
                    }
                    scratch.write_bits(0, 2);
                    scratch.write_var(shooter);
                    scratch.write_bits(q_angle(angle, 12), 12);
                }
                Event::GrenadeThrown { from, to, fuse, .. } => {
                    if !near(from, 30.0) && !near(to, 30.0) {
                        continue;
                    }
                    scratch.write_bits(1, 2);
                    write_pos(&mut scratch, from);
                    write_pos(&mut scratch, to);
                    scratch.write_var((fuse * 10.0).round() as u32);
                }
                Event::Explosion { pos, radius } => {
                    if !near(pos, 30.0) {
                        continue;
                    }
                    scratch.write_bits(2, 2);
                    write_pos(&mut scratch, pos);
                    scratch.write_bits((radius * 4.0).round().min(255.0) as u32, 8);
                }
                Event::Kill { killer, victim } => {
                    scratch.write_bits(3, 2);
                    scratch.write_var(killer);
                    scratch.write_var(victim);
                    scratch.write_bits(world.soldiers[killer as usize].team as u32, 1);
                }
                Event::PointCaptured { .. } | Event::RoundOver { .. } | Event::RoundStarted { .. } => continue,
            }
            if scratch.len_bits() as i64 > left {
                continue;
            }
            left -= scratch.len_bits() as i64;
            events_w.append(&scratch);
            event_count += 1;
        }

        // Чанки карты, которые клиент ещё не подтвердил.
        let mut chunks_w = BitWriter::new();
        let mut sent_chunks = Vec::new();
        for (chunk, &rev) in world.map.chunk_revisions.iter().enumerate() {
            if rev <= self.chunk_acked[chunk] {
                continue;
            }
            let (sent_rev, sent_tick) = self.chunk_sent[chunk];
            if sent_rev == rev && tick.wrapping_sub(sent_tick) < cfg.chunk_resend_ticks {
                continue; // уже в пути
            }
            let mut scratch = BitWriter::new();
            scratch.write_var(chunk as u32);
            scratch.write_var(rev);
            write_rle(&mut scratch, &world.map.chunk_cells(chunk));
            // Один чанк отправим даже сверх бюджета, иначе большой чанк не уйдёт никогда.
            if scratch.len_bits() as i64 > left && !sent_chunks.is_empty() {
                break;
            }
            left -= scratch.len_bits() as i64;
            chunks_w.append(&scratch);
            sent_chunks.push((chunk as u32, rev));
            self.chunk_sent[chunk] = (rev, tick);
            if left <= 0 {
                break;
            }
        }

        w.append(&removed_w);
        w.append(&entities_w);
        w.write_var(event_count);
        w.append(&events_w);
        w.write_var(sent_chunks.len() as u32);
        w.append(&chunks_w);

        // Запоминаем, что теперь (после этого снимка) знает клиент.
        let mut view: View = base_view.iter().filter(|(id, _)| removed.binary_search(id).is_err()).copied().collect();
        for &(id, s, _) in &selected {
            match view.binary_search_by_key(&id, |e| e.0) {
                Ok(i) => view[i].1 = s,
                Err(i) => view.insert(i, (id, s)),
            }
        }
        self.history[tick as usize % HISTORY] = Some(Sent { tick, view, chunks: sent_chunks });
        w.finish()
    }
}

fn write_self(w: &mut BitWriter, me: &Soldier) {
    w.write_bool(me.alive);
    w.write_bits((me.health.clamp(0.0, 1.0) * 100.0).round() as u32, 7);
    w.write_var(me.ammo);
    w.write_bool(me.reload_timer > 0.0);
    w.write_bits(me.grenades.min(7), 3);
    w.write_var((me.respawn_timer.max(0.0) * 10.0).round() as u32);
    w.write_var(me.kills);
}

fn write_gap(w: &mut BitWriter, prev: &mut Option<u32>, id: u32) {
    let gap = match *prev {
        Some(p) => id - p - 1,
        None => id,
    };
    w.write_var(gap);
    *prev = Some(id);
}

fn read_gap(r: &mut BitReader, prev: &mut Option<u32>) -> Result<u32, Truncated> {
    let gap = r.read_var()?;
    let id = match *prev {
        Some(p) => p.checked_add(gap + 1).ok_or(Truncated)?,
        None => gap,
    };
    *prev = Some(id);
    Ok(id)
}

/// Новый солдат — целиком; известный — только изменившиеся поля,
/// а небольшой сдвиг позиции — 10 битами вместо 16.
fn encode_entity(w: &mut BitWriter, s: NetSoldier, base: Option<NetSoldier>) {
    let Some(b) = base else {
        w.write_bits(s.x as u32, 16);
        w.write_bits(s.y as u32, 16);
        w.write_bits(s.facing as u32, 8);
        w.write_bits(s.flags as u32, 8);
        return;
    };
    let moved = s.x != b.x || s.y != b.y;
    w.write_bool(moved);
    if moved {
        let dx = s.x as i32 - b.x as i32;
        let dy = s.y as i32 - b.y as i32;
        let small = dx.abs() < 511 && dy.abs() < 511;
        w.write_bool(small);
        if small {
            w.write_signed(dx, 10);
            w.write_signed(dy, 10);
        } else {
            w.write_bits(s.x as u32, 16);
            w.write_bits(s.y as u32, 16);
        }
    }
    w.write_bool(s.facing != b.facing);
    if s.facing != b.facing {
        w.write_bits(s.facing as u32, 8);
    }
    w.write_bool(s.flags != b.flags);
    if s.flags != b.flags {
        w.write_bits(s.flags as u32, 8);
    }
}

fn decode_entity(r: &mut BitReader, base: Option<NetSoldier>) -> Result<NetSoldier, Truncated> {
    let Some(mut s) = base else {
        return Ok(NetSoldier {
            x: r.read_bits(16)? as u16,
            y: r.read_bits(16)? as u16,
            facing: r.read_bits(8)? as u8,
            flags: r.read_bits(8)? as u8,
        });
    };
    if r.read_bool()? {
        if r.read_bool()? {
            s.x = (s.x as i32 + r.read_signed(10)?) as u16;
            s.y = (s.y as i32 + r.read_signed(10)?) as u16;
        } else {
            s.x = r.read_bits(16)? as u16;
            s.y = r.read_bits(16)? as u16;
        }
    }
    if r.read_bool()? {
        s.facing = r.read_bits(8)? as u8;
    }
    if r.read_bool()? {
        s.flags = r.read_bits(8)? as u8;
    }
    Ok(s)
}

/// Сжатие повторами: чанк — это в основном пустые клетки и ровные стены.
fn write_rle(w: &mut BitWriter, cells: &[u8]) {
    let mut runs = Vec::new();
    let mut i = 0;
    while i < cells.len() {
        let v = cells[i];
        let start = i;
        while i < cells.len() && cells[i] == v {
            i += 1;
        }
        runs.push((i - start, v));
    }
    w.write_var(runs.len() as u32);
    for (len, v) in runs {
        w.write_var(len as u32);
        w.write_bits(v as u32, 8);
    }
}

fn read_rle(r: &mut BitReader, max: usize) -> Result<Vec<u8>, Truncated> {
    let runs = r.read_var()?;
    let mut out = Vec::new();
    for _ in 0..runs {
        let len = r.read_var()? as usize;
        let v = r.read_bits(8)? as u8;
        if out.len() + len > max {
            return Err(Truncated);
        }
        out.resize(out.len() + len, v);
    }
    Ok(out)
}

// =========================================================================
// Клиент
// =========================================================================

#[derive(Clone, Debug, Default)]
pub struct SelfState {
    pub alive: bool,
    pub health: f32,
    pub ammo: u32,
    pub reloading: bool,
    pub grenades: u32,
    pub respawn_in: f32,
    pub kills: u32,
}

#[derive(Clone, Debug)]
pub struct RemoteSoldier {
    pub net: NetSoldier,
    /// Позиция из последнего снимка.
    pub pos: Vec2,
    /// Где солдат был нарисован, когда пришёл снимок: от неё плавно едем к `pos`.
    pub prev: Vec2,
}

#[derive(Clone, Debug)]
pub struct RemotePoint {
    pub pos: Vec2,
    pub owner: Option<u8>,
    pub progress: f32,
}

#[derive(Clone, Debug)]
pub struct Tracer {
    pub pos: Vec2,
    pub vel: Vec2,
    pub ttl: f32,
}

#[derive(Clone, Debug)]
pub struct RemoteGrenade {
    pub pos: Vec2,
    pub vel: Vec2,
    pub flight: f32,
    pub fuse: f32,
}

/// Убитый солдат: где упал. Тело лежит, пока солдат не возродится.
#[derive(Clone, Debug)]
pub struct Corpse {
    pub id: u32,
    pub pos: Vec2,
    pub facing: f32,
    pub team: u8,
    pub age: f32,
}

/// Сколько лежит тело у клиента: столько же, сколько солдат ждёт возрождения.
const CORPSE_TIME: f32 = 6.0;

#[derive(Clone, Debug)]
pub struct KillNote {
    pub killer: u32,
    pub victim: u32,
    pub killer_team: u8,
    pub age: f32,
}

#[derive(Debug, PartialEq)]
pub enum SnapshotError {
    Truncated,
    /// Снимок посчитан от тика, которого у нас уже (или ещё) нет.
    MissingBaseline,
}

impl From<Truncated> for SnapshotError {
    fn from(_: Truncated) -> Self {
        SnapshotError::Truncated
    }
}

/// Что слышно: события боя с местом — для звука. Клиент забирает их каждый кадр.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Heard {
    Shot { pos: Vec2, shooter: u32, team: u8 },
    Throw { pos: Vec2 },
    Blast { pos: Vec2, radius: f32 },
}

/// Больше событий в очереди не держим: если звук их не забирает, старые выбрасываются.
pub const MAX_HEARD: usize = 256;

/// Мир глазами клиента: только то, что прислал сервер, плюс эффекты.
pub struct RemoteWorld {
    pub welcome: Welcome,
    pub map: Map,
    pub bases: [Vec2; 2],
    pub points: Vec<RemotePoint>,
    pub soldiers: Vec<Option<RemoteSoldier>>,
    pub me: SelfState,
    pub score: [u32; 2],
    /// Последний разобранный тик. Его клиент и подтверждает.
    pub tick: u32,
    pub tracers: Vec<Tracer>,
    pub grenades: Vec<RemoteGrenade>,
    /// Взрывы: позиция, радиус, сколько секунд назад.
    pub blasts: Vec<(Vec2, f32, f32)>,
    pub kill_feed: VecDeque<KillNote>,
    pub corpses: Vec<Corpse>,
    /// Атмосфера карты — для картинки.
    pub atmosphere: Atmosphere,
    /// Оружие и классы — из своих `data/weapons.ron` и `data/factions.ron`.
    pub arsenal: Arsenal,
    /// Раунд по последнему снимку.
    pub round: Round,
    /// Таблица бойцов (приходит раз в секунду).
    pub board: Vec<BoardEntry>,
    /// Время клиента, которое вернул сервер в последнем снимке.
    pub echo_time: u32,
    /// Когда (по часам клиента, с) пришёл последний снимок.
    pub snapshot_time: f64,
    /// Где свой солдат на экране (клиент ведёт его сам, см. `Pilot`) — оттуда
    /// и вылетают его трассеры.
    pub own_pos: Option<Vec2>,
    /// Сколько своих выстрелов пришло — для отдачи прицела.
    pub own_shots: u32,
    /// Что слышно с прошлого кадра (см. [`Heard`]).
    pub heard: Vec<Heard>,
    history: Vec<Option<(u32, View)>>,
}

impl RemoteWorld {
    /// Мир по приветствию сервера. Карту из файла ищем в `maps_dir`, оружие — в папке
    /// данных над ней, и сверяем отпечатки с сервером.
    pub fn new(welcome: Welcome, maps_dir: &Path) -> Result<Self, String> {
        let data_dir = maps_dir.parent().unwrap_or(Path::new("."));
        let arsenal = if Arsenal::exists(data_dir) { Arsenal::load(data_dir)? } else { Arsenal::default() };
        if arsenal.fingerprint() != welcome.arsenal_fingerprint {
            return Err("оружие у тебя не такое, как на сервере (data/weapons.ron, data/factions.ron)".into());
        }
        let (map, layout, atmosphere) = if welcome.map_name.is_empty() {
            let (map, layout) = mapgen::generate(welcome.map_size, welcome.capture_points, welcome.seed);
            (map, layout, Atmosphere::default())
        } else {
            let name = &welcome.map_name;
            let file = MapFile::load(&maps_dir.join(name)).map_err(|e| format!("нет карты «{name}»: {e}"))?;
            if file.fingerprint() != welcome.map_fingerprint {
                return Err(format!("карта «{name}» у тебя не такая, как на сервере"));
            }
            (file.map, file.layout, file.atmosphere)
        };
        let points = layout
            .capture_points
            .iter()
            .map(|&pos| RemotePoint { pos, owner: None, progress: 0.0 })
            .collect();
        Ok(RemoteWorld {
            soldiers: vec![None; welcome.soldiers_total as usize],
            welcome,
            map,
            bases: layout.bases,
            points,
            me: SelfState::default(),
            score: [0; 2],
            tick: 0,
            tracers: Vec::new(),
            grenades: Vec::new(),
            blasts: Vec::new(),
            kill_feed: VecDeque::new(),
            corpses: Vec::new(),
            atmosphere,
            arsenal,
            round: Round::default(),
            board: Vec::new(),
            echo_time: 0,
            snapshot_time: 0.0,
            own_pos: None,
            own_shots: 0,
            heard: Vec::new(),
            history: (0..HISTORY).map(|_| None).collect(),
        })
    }

    pub fn dt(&self) -> f32 {
        1.0 / self.welcome.tick_rate.max(1) as f32
    }

    /// Где рисовать солдата сейчас: плавно между прошлым и новым снимком.
    pub fn render_pos(&self, s: &RemoteSoldier, now: f64) -> Vec2 {
        let t = (((now - self.snapshot_time) as f32) / self.dt()).clamp(0.0, 1.0);
        s.prev.lerp(s.pos, t)
    }

    /// Имя бойца: у игрока — его имя, у ИИ — номер.
    pub fn name_of(&self, soldier: u32) -> String {
        match self.board.iter().find(|e| e.soldier == soldier && !e.name.is_empty()) {
            Some(e) => e.name.clone(),
            None => format!("#{soldier}"),
        }
    }

    /// Оружие своего солдата.
    pub fn my_weapon(&self) -> &WeaponSpec {
        self.arsenal.weapon_of(self.welcome.team, self.welcome.soldier)
    }

    pub fn my_soldier(&self) -> Option<&RemoteSoldier> {
        self.soldiers.get(self.welcome.soldier as usize)?.as_ref()
    }

    /// Разбирает снимок (данные после заголовка). Устаревший снимок молча пропускается.
    pub fn apply_snapshot(&mut self, data: &[u8], now: f64) -> Result<(), SnapshotError> {
        let mut r = BitReader::new(data);
        let tick = r.read_bits(32)?;
        if tick <= self.tick {
            return Ok(());
        }
        let base_delta = r.read_var()?;
        let base_view: View = if base_delta == 0 {
            View::new()
        } else {
            let base_tick = tick.wrapping_sub(base_delta);
            match &self.history[base_tick as usize % HISTORY] {
                Some((t, view)) if *t == base_tick => view.clone(),
                _ => return Err(SnapshotError::MissingBaseline),
            }
        };
        let echo_time = r.read_bits(32)?;

        let me = SelfState {
            alive: r.read_bool()?,
            health: r.read_bits(7)? as f32 / 100.0,
            ammo: r.read_var()?,
            reloading: r.read_bool()?,
            grenades: r.read_bits(3)?,
            respawn_in: r.read_var()? as f32 / 10.0,
            kills: r.read_var()?,
        };
        let score = [r.read_var()?, r.read_var()?];
        let point_count = r.read_var()? as usize;
        let mut points = Vec::with_capacity(point_count);
        for _ in 0..point_count {
            let owner = match r.read_bits(2)? {
                0 => None,
                t => Some((t - 1) as u8),
            };
            let progress = r.read_signed(8)? as f32 / 127.0;
            points.push((owner, progress));
        }
        let round = read_round(&mut r)?;
        let board = if r.read_bool()? { Some(read_board(&mut r)?) } else { None };

        let total = self.soldiers.len() as u32;
        let mut view = base_view;
        let removed_count = r.read_var()?;
        let mut prev = None;
        for _ in 0..removed_count {
            let id = read_gap(&mut r, &mut prev)?;
            if let Ok(i) = view.binary_search_by_key(&id, |e| e.0) {
                view.remove(i);
            }
        }
        let entity_count = r.read_var()?;
        let mut prev = None;
        for _ in 0..entity_count {
            let id = read_gap(&mut r, &mut prev)?;
            if id >= total {
                return Err(SnapshotError::Truncated);
            }
            match view.binary_search_by_key(&id, |e| e.0) {
                Ok(i) => view[i].1 = decode_entity(&mut r, Some(view[i].1))?,
                Err(i) => {
                    let s = decode_entity(&mut r, None)?;
                    view.insert(i, (id, s));
                }
            }
        }

        enum Fx {
            Shot(u32, f32),
            Grenade(Vec2, Vec2, f32),
            Blast(Vec2, f32),
            Kill(u32, u32, u8),
        }
        let event_count = r.read_var()?;
        let mut effects = Vec::with_capacity(event_count as usize);
        for _ in 0..event_count {
            effects.push(match r.read_bits(2)? {
                0 => Fx::Shot(r.read_var()?, dq_angle(r.read_bits(12)?, 12)),
                1 => Fx::Grenade(read_pos(&mut r)?, read_pos(&mut r)?, r.read_var()? as f32 / 10.0),
                2 => Fx::Blast(read_pos(&mut r)?, r.read_bits(8)? as f32 / 4.0),
                _ => Fx::Kill(r.read_var()?, r.read_var()?, r.read_bits(1)? as u8),
            });
        }

        let chunk_count = r.read_var()?;
        let mut chunks = Vec::with_capacity(chunk_count as usize);
        for _ in 0..chunk_count {
            let chunk = r.read_var()? as usize;
            let rev = r.read_var()?;
            if chunk >= self.map.chunk_revisions.len() {
                return Err(SnapshotError::Truncated);
            }
            let (_, _, w, h) = self.map.chunk_rect(chunk);
            chunks.push((chunk, rev, read_rle(&mut r, (w * h) as usize)?));
        }

        // Снимок разобран целиком — применяем.
        // Сначала тела: убитый пропадёт из вида, а нам нужно его последнее место.
        for fx in &effects {
            if let Fx::Kill(_, victim, _) = *fx
                && let Some(Some(s)) = self.soldiers.get(victim as usize)
            {
                self.corpses.push(Corpse { id: victim, pos: s.pos, facing: s.net.facing(), team: s.net.team(), age: 0.0 });
            }
        }
        let t = (((now - self.snapshot_time) as f32) / self.dt()).clamp(0.0, 1.0);
        for (id, s) in &view {
            let slot = &mut self.soldiers[*id as usize];
            let shown = slot.as_ref().map(|old| old.prev.lerp(old.pos, t));
            let pos = s.pos();
            *slot = Some(RemoteSoldier { net: *s, pos, prev: shown.unwrap_or(pos) });
        }
        // Кого нет в снимке — тех клиент больше не видит.
        let mut in_view = view.iter().map(|e| e.0).peekable();
        for (id, slot) in self.soldiers.iter_mut().enumerate() {
            if in_view.peek() == Some(&(id as u32)) {
                in_view.next();
            } else {
                *slot = None;
            }
        }

        let radius = self.welcome.soldier_radius;
        for fx in effects {
            match fx {
                Fx::Shot(shooter, angle) => {
                    let own = shooter == self.welcome.soldier;
                    if own {
                        self.own_shots += 1;
                    }
                    let from = match self.soldiers.get(shooter as usize) {
                        Some(Some(s)) => Some(if own { self.own_pos.unwrap_or(s.pos) } else { s.pos }),
                        _ => None,
                    };
                    if let Some(from) = from {
                        let team = self.soldiers[shooter as usize].as_ref().map_or(self.welcome.team, |s| s.net.team());
                        self.heard.push(Heard::Shot { pos: from, shooter, team });
                        let weapon = self.arsenal.weapon_of(team, shooter);
                        let (speed, ttl) = (weapon.projectile_speed, weapon.range * 1.2 / weapon.projectile_speed);
                        let dir = Vec2::new(angle.cos(), angle.sin());
                        self.tracers.push(Tracer { pos: from + dir * (radius + 0.05), vel: dir * speed, ttl });
                    }
                }
                Fx::Grenade(from, to, fuse) => {
                    let d = to - from;
                    let throw = self.welcome.grenade_speed;
                    self.heard.push(Heard::Throw { pos: from });
                    self.grenades.push(RemoteGrenade {
                        pos: from,
                        vel: d.normalize_or_zero() * throw,
                        flight: d.length() / throw,
                        fuse,
                    });
                }
                Fx::Blast(pos, radius) => {
                    self.blasts.push((pos, radius, 0.0));
                    self.heard.push(Heard::Blast { pos, radius });
                }
                Fx::Kill(killer, victim, killer_team) => {
                    self.kill_feed.push_front(KillNote { killer, victim, killer_team, age: 0.0 });
                    self.kill_feed.truncate(6);
                }
            }
        }
        if self.heard.len() > MAX_HEARD {
            let extra = self.heard.len() - MAX_HEARD;
            self.heard.drain(..extra);
        }
        for (chunk, rev, cells) in chunks {
            if rev > self.map.chunk_revisions[chunk] {
                self.map.set_chunk_cells(chunk, &cells, rev);
            }
        }
        for (p, (owner, progress)) in self.points.iter_mut().zip(points) {
            p.owner = owner;
            p.progress = progress;
        }

        // Новый раунд: карта и точки вернутся сами (чанки и снимок), а тела и эффекты
        // прошлого раунда убираем.
        if round.number != self.round.number {
            self.corpses.clear();
            self.tracers.clear();
            self.grenades.clear();
            self.blasts.clear();
        }
        self.round = round;
        if let Some(board) = board {
            self.board = board;
        }
        self.me = me;
        self.score = score;
        self.echo_time = echo_time;
        self.tick = tick;
        self.snapshot_time = now;
        self.history[tick as usize % HISTORY] = Some((tick, view));
        Ok(())
    }

    /// Двигает трассеры и гранаты, старит взрывы. Вызывать каждый кадр.
    pub fn advance_effects(&mut self, dt: f32) {
        let map = &self.map;
        self.tracers.retain_mut(|t| {
            let next = t.pos + t.vel * dt;
            t.ttl -= dt;
            if t.ttl <= 0.0 || map.cast(t.pos, next).is_some() {
                return false;
            }
            t.pos = next;
            true
        });
        self.grenades.retain_mut(|g| {
            g.fuse -= dt;
            if g.flight > 0.0 {
                g.flight -= dt;
                let step = g.vel * dt;
                if map.is_solid_at(g.pos + Vec2::new(step.x, 0.0)) {
                    g.vel.x = -g.vel.x * 0.4;
                } else {
                    g.pos.x += step.x;
                }
                if map.is_solid_at(g.pos + Vec2::new(0.0, step.y)) {
                    g.vel.y = -g.vel.y * 0.4;
                } else {
                    g.pos.y += step.y;
                }
            }
            g.fuse > 0.0
        });
        for b in &mut self.blasts {
            b.2 += dt;
        }
        self.blasts.retain(|b| b.2 < 0.6);
        for k in &mut self.kill_feed {
            k.age += dt;
        }
        for c in &mut self.corpses {
            c.age += dt;
        }
        self.corpses.retain(|c| c.age < CORPSE_TIME);
        while self.kill_feed.back().is_some_and(|k| k.age > 8.0) {
            self.kill_feed.pop_back();
        }
    }
}

/// Раунд в снимке: номер, фаза, победитель, время фазы и удержание (в десятых секунды).
fn write_round(w: &mut BitWriter, round: &Round) {
    w.write_var(round.number);
    match round.phase {
        Phase::Playing => w.write_bool(false),
        Phase::Over { winner, reason } => {
            w.write_bool(true);
            w.write_bits(winner.map_or(0, |t| t as u32 + 1), 2);
            w.write_bool(reason == EndReason::TimeUp);
        }
    }
    let tenths = |s: f32| (s.max(0.0) * 10.0) as u32;
    w.write_var(tenths(round.time));
    w.write_var(tenths(round.hold[0]));
    w.write_var(tenths(round.hold[1]));
}

fn read_round(r: &mut BitReader) -> Result<Round, Truncated> {
    let number = r.read_var()?;
    let phase = if r.read_bool()? {
        let winner = match r.read_bits(2)? {
            0 => None,
            t => Some((t - 1) as u8),
        };
        let reason = if r.read_bool()? { EndReason::TimeUp } else { EndReason::AllPoints };
        Phase::Over { winner, reason }
    } else {
        Phase::Playing
    };
    let time = r.read_var()? as f32 / 10.0;
    let hold = [r.read_var()? as f32 / 10.0, r.read_var()? as f32 / 10.0];
    Ok(Round { number, phase, time, hold })
}

/// Строка таблицы бойцов. Имя есть только у игроков.
#[derive(Clone, Debug, PartialEq)]
pub struct BoardEntry {
    pub soldier: u32,
    pub team: u8,
    pub kills: u32,
    pub deaths: u32,
    pub name: String,
}

/// Сколько лучших бойцов ИИ каждой команды попадает в таблицу.
const BOARD_AI: usize = 5;

/// Таблица бойцов: все игроки (`names` — их солдаты и имена) и лучшие из ИИ,
/// у каждой команды по убийствам.
pub fn board(world: &World, names: &[(u32, String)]) -> Vec<BoardEntry> {
    let mut out = Vec::new();
    for team in 0..2u8 {
        let entry = |i: usize, name: String| {
            let s = &world.soldiers[i];
            BoardEntry { soldier: i as u32, team, kills: s.kills, deaths: s.deaths, name }
        };
        let mut rows: Vec<BoardEntry> = names
            .iter()
            .filter(|(i, _)| world.soldiers.get(*i as usize).is_some_and(|s| s.team == team))
            .map(|(i, name)| entry(*i as usize, name.clone()))
            .collect();
        let mut ai: Vec<usize> = (0..world.soldiers.len())
            .filter(|&i| {
                let s = &world.soldiers[i];
                s.team == team && s.kills > 0 && !names.iter().any(|(p, _)| *p as usize == i)
            })
            .collect();
        ai.sort_by_key(|&i| std::cmp::Reverse(world.soldiers[i].kills));
        rows.extend(ai.into_iter().take(BOARD_AI).map(|i| entry(i, String::new())));
        rows.sort_by_key(|e| std::cmp::Reverse(e.kills));
        out.extend(rows);
    }
    out
}

pub fn write_board(w: &mut BitWriter, board: &[BoardEntry]) {
    w.write_var(board.len() as u32);
    for e in board {
        w.write_var(e.soldier);
        w.write_bits(e.team as u32, 1);
        w.write_var(e.kills);
        w.write_var(e.deaths);
        w.write_str(&e.name, 32);
    }
}

fn read_board(r: &mut BitReader) -> Result<Vec<BoardEntry>, Truncated> {
    let n = r.read_var()?.min(256);
    (0..n)
        .map(|_| {
            Ok(BoardEntry {
                soldier: r.read_var()?,
                team: r.read_bits(1)? as u8,
                kills: r.read_var()?,
                deaths: r.read_var()?,
                name: r.read_str(32)?,
            })
        })
        .collect()
}
