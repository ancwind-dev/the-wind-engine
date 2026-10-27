//! Сервер: принимает клиентов по UDP, раздаёт им солдат, шлёт снимки.
//!
//! Подключение — с проверкой адреса (см. `protocol`): место на сервере достаётся
//! только тому, кто ответил на вызов со своего настоящего адреса, а ввод и выход
//! принимаются только с токеном сессии.

use crate::protocol::{self, Challenge, Hello, InputPacket, Kind, Response, Welcome};
use crate::replication::{ReplicationConfig, ServerReplica, board, write_board};
use crate::traffic::Traffic;
use rayon::prelude::*;
use std::collections::HashMap;
use std::collections::hash_map::RandomState;
use std::hash::BuildHasher;
use std::io::{self, ErrorKind};
use std::net::{IpAddr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};
use wind_sim::World;

/// Как часто (в тиках) рассылать таблицу бойцов.
const BOARD_TICKS: u64 = 20;

/// Клиент, от которого столько не было пакетов, считается отключившимся.
const TIMEOUT: Duration = Duration::from_secs(5);
/// Печать вызова действует столько (и ещё столько же — прошлая), с.
const COOKIE_PERIOD: u64 = 10;
/// Пакетов подключения в секунду с одного IP и всего — больше не отвечаем. На одного
/// игрока — два пакета (привет и ответ), а за одним роутером бывает целая компания.
const HANDSHAKES_PER_IP: u32 = 32;
const HANDSHAKES_TOTAL: u32 = 500;

/// Сколько чужих пакетов сервер отбил — для журнала и проверки.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Guard {
    /// Ввод или выход с неверным токеном — кто-то подставил чужой адрес.
    pub forged: u32,
    /// Ответ на вызов с неверной или просроченной печатью.
    pub bad_cookie: u32,
    /// Слишком часто подключаются — не отвечаем.
    pub throttled: u32,
}

pub struct RemoteClient {
    pub id: u16,
    pub addr: SocketAddr,
    pub name: String,
    pub team: u8,
    pub replica: ServerReplica,
    pub traffic: Traffic,
    /// Токен сессии: без него ввод и выход не принимаем.
    token: u64,
    last_seq: u32,
    last_heard: Instant,
    welcome: Vec<u8>,
}

pub struct NetServer {
    socket: UdpSocket,
    pub clients: Vec<RemoteClient>,
    pub config: ReplicationConfig,
    pub max_clients: usize,
    /// Весь трафик сервера.
    pub traffic: Traffic,
    /// Подключения и отключения — для журнала в интерфейсе.
    pub log: Vec<String>,
    next_id: u16,
    buf: Vec<u8>,
    /// Случайный ключ сервера: им подписываются печати и рождаются токены.
    secret: RandomState,
    minted: u64,
    started: Instant,
    /// Сколько пакетов подключения пришло с каждого IP в этой секунде, и всего.
    handshakes: HashMap<IpAddr, u32>,
    handshakes_total: u32,
    handshake_second: Instant,
    pub guard: Guard,
}

impl NetServer {
    pub fn bind(addr: impl ToSocketAddrs, config: ReplicationConfig) -> io::Result<Self> {
        let socket = UdpSocket::bind(addr)?;
        socket.set_nonblocking(true)?;
        Ok(NetServer {
            socket,
            clients: Vec::new(),
            config,
            max_clients: 24,
            traffic: Traffic::default(),
            log: Vec::new(),
            next_id: 1,
            buf: vec![0; protocol::MAX_PACKET],
            secret: RandomState::new(),
            minted: 0,
            started: Instant::now(),
            handshakes: HashMap::new(),
            handshakes_total: 0,
            handshake_second: Instant::now(),
            guard: Guard::default(),
        })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.socket.local_addr()
    }

    /// Читает всё, что пришло, и отключает молчащих. Вызывать перед тиком.
    pub fn poll(&mut self, world: &mut World) {
        loop {
            match self.socket.recv_from(&mut self.buf) {
                Ok((n, addr)) => {
                    let packet = self.buf[..n].to_vec();
                    self.traffic.on_recv(n);
                    self.handle(&packet, addr, world);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                // Windows так сообщает, что прошлый пакет ушёл на закрытый порт.
                Err(e) if e.kind() == ErrorKind::ConnectionReset => continue,
                Err(e) => {
                    self.log.push(format!("ошибка сокета: {e}"));
                    break;
                }
            }
        }
        let now = Instant::now();
        let mut i = 0;
        while i < self.clients.len() {
            if now.duration_since(self.clients[i].last_heard) > TIMEOUT {
                let c = self.clients.remove(i);
                world.release_control(c.replica.soldier);
                self.log.push(format!("{} отвалился по таймауту", c.name));
            } else {
                i += 1;
            }
        }
    }

    /// Печать вызова для адреса и соли в промежутке времени `period`.
    fn cookie(&self, addr: SocketAddr, salt: u64, period: u64) -> u64 {
        self.secret.hash_one((addr, salt, period, "cookie"))
    }

    fn period(&self) -> u64 {
        self.started.elapsed().as_secs() / COOKIE_PERIOD
    }

    /// Случайный токен сессии (не ноль).
    fn mint(&mut self) -> u64 {
        self.minted += 1;
        self.secret.hash_one((self.minted, self.started.elapsed().as_nanos(), "token")).max(1)
    }

    /// Можно ли ответить на пакет подключения с этого адреса: не больше
    /// `HANDSHAKES_PER_IP` в секунду с одного IP и `HANDSHAKES_TOTAL` всего.
    fn allow_handshake(&mut self, addr: SocketAddr) -> bool {
        if self.handshake_second.elapsed() >= Duration::from_secs(1) {
            self.handshake_second = Instant::now();
            self.handshakes.clear();
            self.handshakes_total = 0;
        }
        let n = self.handshakes.entry(addr.ip()).or_insert(0);
        *n += 1;
        self.handshakes_total += 1;
        let ok = *n <= HANDSHAKES_PER_IP && self.handshakes_total <= HANDSHAKES_TOTAL;
        if !ok {
            self.guard.throttled += 1;
        }
        ok
    }

    fn handle(&mut self, packet: &[u8], addr: SocketAddr, world: &mut World) {
        let Some((kind, data)) = protocol::parse(packet) else { return };
        let index = self.clients.iter().position(|c| c.addr == addr);
        match kind {
            Kind::Hello => {
                // На вызов ничего не запоминаем: место выделим, когда адрес подтвердится.
                let Some(salt) = protocol::decode_hello(data) else { return };
                if index.is_some() || !self.allow_handshake(addr) {
                    return;
                }
                let challenge = Challenge { salt, cookie: self.cookie(addr, salt, self.period()) };
                self.send_to(addr, &challenge.encode());
            }
            Kind::Response => {
                let Ok(response) = Response::decode(data) else { return };
                if let Some(i) = index {
                    // Наш ответ потерялся — повторяем.
                    let welcome = self.clients[i].welcome.clone();
                    self.send(i, &welcome);
                    return;
                }
                if !self.allow_handshake(addr) {
                    return;
                }
                let Challenge { salt, cookie } = response.challenge;
                let period = self.period();
                let fresh = [period, period.saturating_sub(1)].iter().any(|&p| self.cookie(addr, salt, p) == cookie);
                if !fresh {
                    self.guard.bad_cookie += 1;
                    return;
                }
                self.accept(response.hello, addr, world);
            }
            Kind::Input | Kind::Bye => {
                // Только свой клиент и только со своим токеном: иначе кто-то подставил
                // его адрес, чтобы управлять чужим солдатом или выкинуть игрока.
                let Some(i) = index else { return };
                let Some((token, data)) = protocol::unseal(data) else { return };
                if token != self.clients[i].token {
                    self.guard.forged += 1;
                    return;
                }
                self.clients[i].last_heard = Instant::now();
                self.clients[i].traffic.on_recv(packet.len());
                if kind == Kind::Bye {
                    let c = self.clients.remove(i);
                    world.release_control(c.replica.soldier);
                    self.log.push(format!("{} отключился", c.name));
                    return;
                }
                let Ok(input) = InputPacket::decode(data) else { return };
                let c = &mut self.clients[i];
                c.replica.on_ack(input.ack_tick);
                if input.seq > c.last_seq {
                    c.last_seq = input.seq;
                    c.replica.echo_time = input.time_ms;
                    world.set_input(c.replica.soldier, input.input);
                }
            }
            Kind::Welcome | Kind::Snapshot | Kind::Reject | Kind::Challenge => {}
        }
    }

    fn accept(&mut self, hello: Hello, addr: SocketAddr, world: &mut World) {
        if self.clients.len() >= self.max_clients {
            self.send_to(addr, &protocol::reject("сервер заполнен"));
            return;
        }
        let team = hello.team.unwrap_or_else(|| {
            let blue = self.clients.iter().filter(|c| c.team == 0).count();
            let red = self.clients.len() - blue;
            if blue <= red { 0 } else { 1 }
        });
        let Some(soldier) = world.take_control(team) else {
            self.send_to(addr, &protocol::reject("в команде нет свободных солдат"));
            return;
        };
        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1).max(1);
        let token = self.mint();
        let welcome = Welcome { token, ..welcome_for(world, id, soldier, &self.config) }.encode();
        let name = if hello.name.trim().is_empty() { format!("Игрок {id}") } else { hello.name };
        self.log.push(format!("{name} подключился ({addr}), солдат #{soldier}"));
        self.clients.push(RemoteClient {
            id,
            addr,
            name,
            team,
            replica: ServerReplica::new(soldier, world),
            traffic: Traffic::default(),
            token,
            last_seq: 0,
            last_heard: Instant::now(),
            welcome: welcome.clone(),
        });
        let i = self.clients.len() - 1;
        self.send(i, &welcome);
    }

    /// Шлёт каждому клиенту его снимок. Вызывать после тика.
    pub fn send_snapshots(&mut self, world: &World) {
        // Раз в секунду — таблица бойцов (одна на всех): игроки с именами и лучшие из ИИ.
        let board = world.tick.is_multiple_of(BOARD_TICKS).then(|| {
            let names: Vec<(u32, String)> = self.clients.iter().map(|c| (c.replica.soldier, c.name.clone())).collect();
            let mut w = crate::bits::BitWriter::new();
            write_board(&mut w, &board(world, &names));
            w
        });
        // Снимки клиентов независимы — собираем их на всех ядрах.
        let config = &self.config;
        let packets: Vec<Vec<u8>> = self
            .clients
            .par_iter_mut()
            .map(|c| c.replica.build_snapshot(world, config, board.as_ref()))
            .collect();
        for (i, packet) in packets.iter().enumerate() {
            let sealed = protocol::seal(packet, self.clients[i].token);
            self.send(i, &sealed);
        }
        self.traffic.roll();
        for c in &mut self.clients {
            c.traffic.roll();
        }
    }

    fn send(&mut self, client: usize, packet: &[u8]) {
        let addr = self.clients[client].addr;
        self.clients[client].traffic.on_send(packet.len());
        self.send_to(addr, packet);
    }

    fn send_to(&mut self, addr: SocketAddr, packet: &[u8]) {
        self.traffic.on_send(packet.len());
        // Не ушло — не страшно: следующий снимок всё догонит.
        let _ = self.socket.send_to(packet, addr);
    }
}

/// Приветствие для клиента, которому достался солдат `soldier`.
pub fn welcome_for(world: &World, client_id: u16, soldier: u32, config: &ReplicationConfig) -> Welcome {
    let c = &world.config;
    Welcome {
        token: 0,
        client_id,
        soldier,
        team: world.soldiers[soldier as usize].team,
        seed: world.seed,
        map_size: c.map_size,
        capture_points: c.capture_points,
        soldiers_total: world.soldiers.len() as u32,
        tick_rate: c.tick_rate,
        capture_radius: c.capture.radius,
        soldier_radius: c.soldier.radius,
        interest_radius: config.interest_radius,
        grenade_speed: c.grenade.throw_speed,
        soldier_speed: c.soldier.speed,
        arsenal_fingerprint: c.arsenal.fingerprint(),
        vision_range: c.soldier.vision_range,
        hold_time: c.round.hold_time,
        time_limit: c.round.time_limit,
        intermission: c.round.intermission,
        map_name: world.map_file.as_ref().map_or(String::new(), |f| f.0.clone()),
        map_fingerprint: world.map_file.as_ref().map_or(0, |f| f.1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wind_sim::{PlayerInput, SimConfig, Vec2};

    fn world() -> World {
        World::new(SimConfig { map_size: 128, soldiers_per_team: 4, capture_points: 1, ..Default::default() }, 1)
    }

    fn server() -> NetServer {
        NetServer::bind("127.0.0.1:0", ReplicationConfig::default()).unwrap()
    }

    /// Сокет «игрока» — на него сервер шлёт ответы.
    fn player() -> (UdpSocket, SocketAddr) {
        let s = UdpSocket::bind("127.0.0.1:0").unwrap();
        s.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
        let a = s.local_addr().unwrap();
        (s, a)
    }

    fn recv(s: &UdpSocket) -> Option<Vec<u8>> {
        let mut buf = vec![0; protocol::MAX_PACKET];
        s.recv_from(&mut buf).ok().map(|(n, _)| buf[..n].to_vec())
    }

    fn feed(server: &mut NetServer, w: &mut World, packet: &[u8], from: SocketAddr) {
        let packet = packet.to_vec();
        server.handle(&packet, from, w);
    }

    /// Полное подключение: привет → вызов → ответ → приветствие с токеном.
    fn join(server: &mut NetServer, w: &mut World, sock: &UdpSocket, addr: SocketAddr) -> Welcome {
        let hello = protocol::hello(42);
        feed(server, w, &hello, addr);
        let reply = recv(sock).expect("вызов");
        assert!(reply.len() <= hello.len(), "ответ на привет не больше самого привета");
        let (kind, data) = protocol::parse(&reply).unwrap();
        assert_eq!(kind, Kind::Challenge);
        let challenge = Challenge::decode(data).unwrap();
        assert_eq!(challenge.salt, 42);
        assert!(server.clients.is_empty(), "на привет место не выделяем");
        let response = Response { challenge, hello: Hello { name: "Игрок".into(), team: Some(0) } };
        feed(server, w, &response.encode(), addr);
        let (kind, data) = protocol::parse(&recv(sock).expect("приветствие")).map(|(k, d)| (k, d.to_vec())).unwrap();
        assert_eq!(kind, Kind::Welcome);
        Welcome::decode(&data).unwrap()
    }

    fn input(token: u64, seq: u32, fire: bool) -> Vec<u8> {
        let p = InputPacket { seq, ack_tick: 0, time_ms: 1, input: PlayerInput { fire, aim: Vec2::ONE, ..Default::default() } };
        protocol::seal(&p.encode(), token)
    }

    #[test]
    fn honest_client_joins_and_plays() {
        let (mut s, mut w) = (server(), world());
        let (sock, addr) = player();
        let welcome = join(&mut s, &mut w, &sock, addr);
        assert_eq!(s.clients.len(), 1);
        assert_ne!(welcome.token, 0);
        feed(&mut s, &mut w, &input(welcome.token, 1, true), addr);
        let soldier = s.clients[0].replica.soldier as usize;
        assert!(matches!(w.soldiers[soldier].controller, wind_sim::Controller::Player(i) if i.fire), "ввод с токеном принят");
        // Снимки приходят с тем же токеном.
        s.send_snapshots(&w);
        let snap = recv(&sock).unwrap();
        let (kind, data) = protocol::parse(&snap).unwrap();
        assert_eq!(kind, Kind::Snapshot);
        assert_eq!(protocol::unseal(data).unwrap().0, welcome.token);
        assert_eq!(s.guard, Guard::default());
    }

    #[test]
    fn forged_input_and_bye_are_ignored() {
        let (mut s, mut w) = (server(), world());
        let (sock, addr) = player();
        let welcome = join(&mut s, &mut w, &sock, addr);
        // Кто-то подставил адрес игрока, но токена не знает.
        feed(&mut s, &mut w, &input(welcome.token ^ 1, 5, true), addr);
        let soldier = s.clients[0].replica.soldier as usize;
        assert!(matches!(w.soldiers[soldier].controller, wind_sim::Controller::Player(i) if !i.fire), "чужой ввод не принят");
        feed(&mut s, &mut w, &protocol::seal(&protocol::bye(), 12345), addr);
        feed(&mut s, &mut w, &protocol::bye(), addr);
        assert_eq!(s.clients.len(), 1, "игрока не выкинули");
        assert_eq!(s.guard.forged, 2);
        // А свой выход — работает.
        feed(&mut s, &mut w, &protocol::seal(&protocol::bye(), welcome.token), addr);
        assert!(s.clients.is_empty());
    }

    #[test]
    fn spoofed_address_cannot_join() {
        let (mut s, mut w) = (server(), world());
        let (_victim, victim) = player();
        let (attacker_sock, attacker) = player();
        // Атакующий шлёт привет со своего адреса и получает печать...
        feed(&mut s, &mut w, &protocol::hello(7), attacker);
        let (_, data) = protocol::parse(&recv(&attacker_sock).unwrap()).map(|(k, d)| (k, d.to_vec())).unwrap();
        let challenge = Challenge::decode(&data).unwrap();
        // ...но с ней не войти «от имени» жертвы: печать привязана к адресу.
        let response = Response { challenge, hello: Hello { name: "x".into(), team: None } };
        feed(&mut s, &mut w, &response.encode(), victim);
        // Выдуманная печать — тоже нет.
        let fake = Response { challenge: Challenge { salt: 7, cookie: 0xDEAD }, hello: Hello { name: "x".into(), team: None } };
        feed(&mut s, &mut w, &fake.encode(), attacker);
        assert!(s.clients.is_empty());
        assert_eq!(s.guard.bad_cookie, 2);
        // Недобитый привет (меньше 64 байт) — без ответа: иначе ответ больше запроса.
        let short = protocol::hello(7)[..20].to_vec();
        feed(&mut s, &mut w, &short, attacker);
        attacker_sock.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
        assert!(recv(&attacker_sock).is_none());
    }

    #[test]
    fn handshake_flood_is_throttled() {
        let (mut s, mut w) = (server(), world());
        let (sock, addr) = player();
        sock.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
        for k in 0..50 {
            feed(&mut s, &mut w, &protocol::hello(k), addr);
        }
        let mut answered = 0;
        while recv(&sock).is_some() {
            answered += 1;
        }
        assert_eq!(answered, HANDSHAKES_PER_IP);
        assert_eq!(s.guard.throttled, 50 - HANDSHAKES_PER_IP);
    }
}
