//! Пакеты протокола и сжатие чисел.
//!
//! Каждый пакет начинается с 4 байт: «WN», версия протокола, тип пакета.
//! Дальше — данные по битам.
//!
//! Подключение — в три шага, чтобы нельзя было подставить чужой адрес:
//! 1. Клиент → `Hello`: случайная «соль», пакет добит до `HELLO_MIN` байт.
//! 2. Сервер → `Challenge`: соль, печать (ключевой хэш адреса, соли и времени) и
//!    постоянный открытый ключ сервера. Сервер ничего не запоминает, а ответ не больше
//!    запроса — подставив чужой адрес, атаку не усилить.
//! 3. Клиент → `Response`: соль, печать, имя, открытый ключ и пароль, зашифрованный
//!    для ключа сервера. Печать сходится — адрес настоящий; пароль верный — сервер
//!    выделяет место и шлёт `Welcome` с ключом сессии и именем сервера.
//!
//! Дальше `Input`, `Bye` клиента и `Snapshot` сервера зашифрованы ключами сессии
//! (см. `crypto`): прочитать их со стороны нельзя, а подделанный или изменённый пакет
//! не расшифруется.

use crate::bits::{BitReader, BitWriter, Truncated};
use std::f32::consts::TAU;
use wind_sim::{PlayerInput, Vec2};

pub const MAGIC: [u8; 2] = *b"WN";
pub const VERSION: u8 = 15;
pub const DEFAULT_PORT: u16 = 7777;
pub const MAX_NAME: usize = 24;
/// Пароль сервера и имя сервера — не длиннее, байт.
pub const MAX_PASSWORD: usize = 64;
pub const MAX_SERVER_NAME: usize = 48;
/// Больше этого UDP-пакет не читаем.
pub const MAX_PACKET: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Kind {
    Hello = 1,
    Input = 2,
    Bye = 3,
    Response = 4,
    Welcome = 10,
    Snapshot = 11,
    Reject = 12,
    Challenge = 13,
    /// Сервер выкинул игрока (зашифрован ключом сессии — не подделать).
    Kick = 14,
}

impl Kind {
    fn from_u8(v: u8) -> Option<Kind> {
        Some(match v {
            1 => Kind::Hello,
            2 => Kind::Input,
            3 => Kind::Bye,
            4 => Kind::Response,
            10 => Kind::Welcome,
            11 => Kind::Snapshot,
            12 => Kind::Reject,
            13 => Kind::Challenge,
            14 => Kind::Kick,
            _ => return None,
        })
    }
}

pub fn begin(kind: Kind) -> BitWriter {
    let mut w = BitWriter::new();
    for b in [MAGIC[0], MAGIC[1], VERSION, kind as u8] {
        w.write_bits(b as u32, 8);
    }
    w
}

/// Тип пакета и данные после заголовка. Чужие и битые пакеты — `None`.
pub fn parse(packet: &[u8]) -> Option<(Kind, &[u8])> {
    if packet.len() < 4 || packet[..2] != MAGIC || packet[2] != VERSION {
        return None;
    }
    Some((Kind::from_u8(packet[3])?, &packet[4..]))
}

/// Длина заголовка пакета, байт.
pub const HEADER: usize = 4;

// --- Подключение -------------------------------------------------------------

/// `Hello` не короче: ответ сервера (`Challenge`) не больше запроса.
pub const HELLO_MIN: usize = 64;

pub fn hello(salt: u64) -> Vec<u8> {
    let mut w = begin(Kind::Hello);
    w.write_u64(salt);
    let mut p = w.finish();
    p.resize(HELLO_MIN, 0);
    p
}

/// Соль из `Hello`; короткий (недобитый) `Hello` не принимаем.
pub fn decode_hello(data: &[u8]) -> Option<u64> {
    if data.len() + HEADER < HELLO_MIN {
        return None;
    }
    BitReader::new(data).read_u64().ok()
}

/// Вызов сервера: соль клиента, печать и постоянный ключ сервера.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Challenge {
    pub salt: u64,
    pub cookie: u64,
    /// Постоянный открытый ключ сервера — для пароля.
    pub server_key: [u8; 32],
}

impl Challenge {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = begin(Kind::Challenge);
        w.write_u64(self.salt);
        w.write_u64(self.cookie);
        write_key(&mut w, &self.server_key);
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Result<Self, Truncated> {
        let mut r = BitReader::new(data);
        Ok(Challenge { salt: r.read_u64()?, cookie: r.read_u64()?, server_key: read_key(&mut r)? })
    }

    /// Доп. данные для пароля: пароль привязан к этому вызову.
    pub fn aad(&self) -> [u8; 16] {
        let mut a = [0u8; 16];
        a[..8].copy_from_slice(&self.salt.to_le_bytes());
        a[8..].copy_from_slice(&self.cookie.to_le_bytes());
        a
    }
}

/// Ответ клиента на вызов: печать, с чем пришёл и открытый ключ для шифрования.
#[derive(Debug, Clone, PartialEq)]
pub struct Response {
    pub challenge: Challenge,
    pub hello: Hello,
    pub key: [u8; 32],
    /// Пароль, зашифрованный для ключа сервера (пустой пароль — тоже зашифрован).
    pub password: Vec<u8>,
}

fn write_key(w: &mut BitWriter, key: &[u8; 32]) {
    for chunk in key.chunks(8) {
        w.write_u64(u64::from_le_bytes(chunk.try_into().unwrap()));
    }
}

fn read_key(r: &mut BitReader) -> Result<[u8; 32], Truncated> {
    let mut key = [0u8; 32];
    for chunk in key.chunks_mut(8) {
        chunk.copy_from_slice(&r.read_u64()?.to_le_bytes());
    }
    Ok(key)
}

impl Response {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = begin(Kind::Response);
        w.write_u64(self.challenge.salt);
        w.write_u64(self.challenge.cookie);
        w.write_str(&self.hello.name, MAX_NAME);
        w.write_bits(self.hello.team.map_or(0, |t| t as u32 + 1), 2);
        write_key(&mut w, &self.key);
        w.write_var(self.password.len() as u32);
        for &b in &self.password {
            w.write_bits(b as u32, 8);
        }
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Result<Self, Truncated> {
        let mut r = BitReader::new(data);
        let challenge = Challenge { salt: r.read_u64()?, cookie: r.read_u64()?, server_key: [0; 32] };
        let name = r.read_str(MAX_NAME)?;
        let team = match r.read_bits(2)? {
            0 => None,
            t => Some((t - 1).min(1) as u8),
        };
        let key = read_key(&mut r)?;
        let len = (r.read_var()? as usize).min(MAX_PASSWORD + 16);
        let password = (0..len).map(|_| r.read_bits(8).map(|b| b as u8)).collect::<Result<_, _>>()?;
        Ok(Response { challenge, hello: Hello { name, team }, key, password })
    }
}

// --- Сжатие чисел ---------------------------------------------------------

/// Позиция хранится в 1/32 метра: 16 бит покрывают карту до 2048 м с точностью 3 см.
pub const POS_SCALE: f32 = 32.0;

pub fn q_pos(v: f32) -> u16 {
    (v * POS_SCALE).round().clamp(0.0, u16::MAX as f32) as u16
}

pub fn dq_pos(q: u16) -> f32 {
    q as f32 / POS_SCALE
}

pub fn q_vec(v: Vec2) -> (u16, u16) {
    (q_pos(v.x), q_pos(v.y))
}

pub fn dq_vec(q: (u16, u16)) -> Vec2 {
    Vec2::new(dq_pos(q.0), dq_pos(q.1))
}

/// Угол в `bits` бит.
pub fn q_angle(a: f32, bits: u32) -> u32 {
    let steps = (1u32 << bits) as f32;
    ((a / TAU).rem_euclid(1.0) * steps).round() as u32 & ((1 << bits) - 1)
}

pub fn dq_angle(q: u32, bits: u32) -> f32 {
    q as f32 / (1u32 << bits) as f32 * TAU
}

pub fn write_pos(w: &mut BitWriter, p: Vec2) {
    let (x, y) = q_vec(p);
    w.write_bits(x as u32, 16);
    w.write_bits(y as u32, 16);
}

pub fn read_pos(r: &mut BitReader) -> Result<Vec2, Truncated> {
    let x = r.read_bits(16)? as u16;
    let y = r.read_bits(16)? as u16;
    Ok(dq_vec((x, y)))
}

// --- Клиент → сервер -------------------------------------------------------

/// С чем клиент приходит на сервер (идёт в `Response`).
#[derive(Debug, Clone, PartialEq)]
pub struct Hello {
    pub name: String,
    /// `None` — в команду, где меньше игроков.
    pub team: Option<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InputPacket {
    /// Номер пакета: старые и задержавшиеся сервер отбрасывает.
    pub seq: u32,
    /// Последний тик, снимок которого клиент получил и разобрал. 0 — ещё ни одного.
    pub ack_tick: u32,
    /// Время клиента в мс; сервер возвращает его в снимке — так считаем пинг.
    pub time_ms: u32,
    pub input: PlayerInput,
}

impl InputPacket {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = begin(Kind::Input);
        w.write_var(self.seq);
        w.write_bits(self.ack_tick, 32);
        w.write_bits(self.time_ms, 32);
        let m = self.input.move_dir.clamp_length_max(1.0) * 127.0;
        w.write_signed(m.x.round() as i32, 8);
        w.write_signed(m.y.round() as i32, 8);
        write_pos(&mut w, self.input.aim);
        w.write_bool(self.input.fire);
        w.write_bool(self.input.grenade);
        w.write_bool(self.input.reload);
        w.write_bool(self.input.walk);
        w.write_bits(self.input.spawn as u32, 5);
        w.write_bool(self.input.pos.is_some());
        if let Some(p) = self.input.pos {
            write_pos(&mut w, p);
        }
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Result<Self, Truncated> {
        let mut r = BitReader::new(data);
        let seq = r.read_var()?;
        let ack_tick = r.read_bits(32)?;
        let time_ms = r.read_bits(32)?;
        let mx = r.read_signed(8)? as f32 / 127.0;
        let my = r.read_signed(8)? as f32 / 127.0;
        let aim = read_pos(&mut r)?;
        let fire = r.read_bool()?;
        let grenade = r.read_bool()?;
        let reload = r.read_bool()?;
        let walk = r.read_bool()?;
        let spawn = r.read_bits(5)? as u8;
        let pos = if r.read_bool()? { Some(read_pos(&mut r)?) } else { None };
        Ok(InputPacket {
            seq,
            ack_tick,
            time_ms,
            input: PlayerInput { move_dir: Vec2::new(mx, my).clamp_length_max(1.0), aim, fire, grenade, reload, walk, spawn, pos },
        })
    }
}

pub fn bye() -> Vec<u8> {
    begin(Kind::Bye).finish()
}

// --- Сервер → клиент -------------------------------------------------------

/// Всё, что клиенту нужно знать о матче. Карту клиент строит сам из seed.
#[derive(Debug, Clone, PartialEq)]
pub struct Welcome {
    /// Открытый ключ сервера для этой сессии (см. `crypto`).
    pub server_key: [u8; 32],
    pub client_id: u16,
    pub soldier: u32,
    pub team: u8,
    pub seed: u64,
    pub map_size: u32,
    pub capture_points: u32,
    pub soldiers_total: u32,
    pub tick_rate: u32,
    pub capture_radius: f32,
    pub soldier_radius: f32,
    pub interest_radius: f32,
    pub grenade_speed: f32,
    /// Скорость шага солдата, м/с — клиент двигает своего солдата сам.
    pub soldier_speed: f32,
    /// Отпечаток оружия и классов (`Arsenal::fingerprint`): у клиента должны быть
    /// те же `weapons.ron` и `factions.ron`, что и на сервере.
    pub arsenal_fingerprint: u64,
    /// Дальность обзора солдата, м — для тумана войны на клиенте.
    pub vision_range: f32,
    /// Правила раунда: сколько держать все точки, длина раунда и перерыва, с.
    pub hold_time: f32,
    pub time_limit: f32,
    pub intermission: f32,
    /// Карта из файла: имя папки в `data/maps` и отпечаток. Пустое имя — карта из seed.
    pub map_name: String,
    pub map_fingerprint: u64,
    /// Как называется сервер.
    pub server_name: String,
}

impl Welcome {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = begin(Kind::Welcome);
        write_key(&mut w, &self.server_key);
        w.write_bits(self.client_id as u32, 16);
        w.write_var(self.soldier);
        w.write_bits(self.team as u32, 1);
        w.write_u64(self.seed);
        w.write_var(self.map_size);
        w.write_var(self.capture_points);
        w.write_var(self.soldiers_total);
        w.write_var(self.tick_rate);
        for v in [
            self.capture_radius,
            self.soldier_radius,
            self.interest_radius,
            self.grenade_speed,
            self.soldier_speed,
            self.vision_range,
            self.hold_time,
            self.time_limit,
            self.intermission,
        ] {
            w.write_f32(v);
        }
        w.write_u64(self.arsenal_fingerprint);
        w.write_str(&self.map_name, 128);
        w.write_u64(self.map_fingerprint);
        w.write_str(&self.server_name, MAX_SERVER_NAME);
        w.finish()
    }

    pub fn decode(data: &[u8]) -> Result<Self, Truncated> {
        let mut r = BitReader::new(data);
        Ok(Welcome {
            server_key: read_key(&mut r)?,
            client_id: r.read_bits(16)? as u16,
            soldier: r.read_var()?,
            team: r.read_bits(1)? as u8,
            seed: r.read_u64()?,
            map_size: r.read_var()?,
            capture_points: r.read_var()?,
            soldiers_total: r.read_var()?,
            tick_rate: r.read_var()?,
            capture_radius: r.read_f32()?,
            soldier_radius: r.read_f32()?,
            interest_radius: r.read_f32()?,
            grenade_speed: r.read_f32()?,
            soldier_speed: r.read_f32()?,
            vision_range: r.read_f32()?,
            hold_time: r.read_f32()?,
            time_limit: r.read_f32()?,
            intermission: r.read_f32()?,
            arsenal_fingerprint: r.read_u64()?,
            map_name: r.read_str(128)?,
            map_fingerprint: r.read_u64()?,
            server_name: r.read_str(MAX_SERVER_NAME)?,
        })
    }
}

pub fn reject(reason: &str) -> Vec<u8> {
    let mut w = begin(Kind::Reject);
    w.write_str(reason, 128);
    w.finish()
}

/// Сервер выкинул игрока: причина (пакет потом шифруется ключом сессии).
pub fn kick(reason: &str) -> Vec<u8> {
    let mut w = begin(Kind::Kick);
    w.write_str(reason, 128);
    w.finish()
}

pub fn decode_reject(data: &[u8]) -> String {
    BitReader::new(data).read_str(128).unwrap_or_else(|_| "отказано".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_round_trip() {
        let p = InputPacket {
            seq: 77,
            ack_tick: 1234,
            time_ms: 99_000,
            input: PlayerInput {
                move_dir: Vec2::new(1.0, 0.0),
                aim: Vec2::new(100.5, 200.25),
                fire: true,
                grenade: false,
                reload: true,
                walk: true,
                spawn: 3,
                pos: Some(Vec2::new(12.0, 34.5)),
            },
        };
        let bytes = p.encode();
        let (kind, payload) = parse(&bytes).unwrap();
        assert_eq!(kind, Kind::Input);
        let back = InputPacket::decode(payload).unwrap();
        assert_eq!(back.seq, 77);
        assert_eq!(back.ack_tick, 1234);
        assert_eq!(back.input.aim, Vec2::new(100.5, 200.25));
        assert!(back.input.fire && !back.input.grenade && back.input.reload && back.input.walk);
        assert!((back.input.move_dir.x - 1.0).abs() < 0.01);
        assert_eq!(back.input.pos, Some(Vec2::new(12.0, 34.5)));
        assert_eq!(back.input.spawn, 3);
    }

    #[test]
    fn handshake_packets_round_trip() {
        let h = hello(0xABCD_0123_4567_89EF);
        assert_eq!(h.len(), HELLO_MIN);
        let (kind, data) = parse(&h).unwrap();
        assert_eq!(kind, Kind::Hello);
        assert_eq!(decode_hello(data), Some(0xABCD_0123_4567_89EF));
        // Недобитый Hello не принимаем.
        assert_eq!(decode_hello(&data[..20]), None);
        let c = Challenge { salt: 7, cookie: 99, server_key: [3; 32] };
        let enc = c.encode();
        assert!(enc.len() <= HELLO_MIN, "ответ не больше запроса");
        assert_eq!(Challenge::decode(parse(&enc).unwrap().1).unwrap(), c);
        let r = Response { challenge: c, hello: Hello { name: "Артём".into(), team: Some(1) }, key: [7; 32], password: vec![1, 2, 3] };
        let back = Response::decode(parse(&r.encode()).unwrap().1).unwrap();
        // Ключ сервера в ответе не повторяется — он уже есть у сервера.
        assert_eq!(back, Response { challenge: Challenge { server_key: [0; 32], ..c }, ..r });
    }

    #[test]
    fn angles() {
        for a in [0.0f32, 1.0, 3.0, -2.0, 6.2] {
            let back = dq_angle(q_angle(a, 12), 12);
            let diff = (back - a.rem_euclid(TAU)).abs();
            assert!(diff < 0.002 || (TAU - diff) < 0.002, "{a} -> {back}");
        }
    }
}
