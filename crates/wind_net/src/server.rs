//! Сервер: принимает клиентов по UDP, раздаёт им солдат, шлёт снимки.
//!
//! Подключение — с проверкой адреса (см. `protocol`): место на сервере достаётся
//! только тому, кто ответил на вызов со своего настоящего адреса. Дальше ввод, выход
//! и снимки зашифрованы ключами сессии (см. `crypto`).

use crate::crypto::{Keypair, Session};
use crate::protocol::{self, Challenge, Hello, InputPacket, Kind, Response, Welcome};
use crate::replication::{ReplicationConfig, ServerReplica, board, write_board};
use crate::traffic::Traffic;
use crate::vision::Vision;
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
/// После стольких неверных паролей подряд IP не пускаем `PASSWORD_LOCKOUT` —
/// подбирать пароль перебором бессмысленно.
const PASSWORD_TRIES: u32 = 5;
const PASSWORD_LOCKOUT: Duration = Duration::from_secs(60);

/// Сколько чужих пакетов сервер отбил — для журнала и проверки.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Guard {
    /// Ввод или выход, который не расшифровался, — подделан или изменён в пути.
    pub forged: u32,
    /// Ответ на вызов с неверной или просроченной печатью.
    pub bad_cookie: u32,
    /// Слишком часто подключаются — не отвечаем.
    pub throttled: u32,
    /// Неверный пароль.
    pub wrong_password: u32,
}

pub struct RemoteClient {
    pub id: u16,
    pub addr: SocketAddr,
    pub name: String,
    pub team: u8,
    pub replica: ServerReplica,
    pub traffic: Traffic,
    /// Ключи сессии: расшифровать ввод, зашифровать снимки.
    session: Session,
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
    started: Instant,
    /// Сколько пакетов подключения пришло с каждого IP в этой секунде, и всего.
    handshakes: HashMap<IpAddr, u32>,
    handshakes_total: u32,
    handshake_second: Instant,
    pub guard: Guard,
    /// Кого видит каждая команда (туман войны на сервере).
    pub vision: Vision,
    /// Как называется сервер — показывается игрокам.
    pub name: String,
    /// Пароль сервера; пустой — без пароля.
    pub password: String,
    /// Запрещённые адреса (баны). Хранит их хозяин сервера (`wind_server`).
    pub banned: Vec<IpAddr>,
    /// Постоянный ключ сервера: им клиенты шифруют пароль.
    identity: Keypair,
    /// Неверные пароли подряд с IP и когда был последний.
    wrong_passwords: HashMap<IpAddr, (u32, Instant)>,
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
            started: Instant::now(),
            handshakes: HashMap::new(),
            handshakes_total: 0,
            handshake_second: Instant::now(),
            guard: Guard::default(),
            vision: Vision::default(),
            name: "The Wind Engine".into(),
            password: String::new(),
            banned: Vec::new(),
            identity: Keypair::generate(),
            wrong_passwords: HashMap::new(),
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
                let challenge = Challenge { salt, cookie: self.cookie(addr, salt, self.period()), server_key: self.identity.public };
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
                let Challenge { salt, cookie, .. } = response.challenge;
                let period = self.period();
                let fresh = [period, period.saturating_sub(1)].iter().any(|&p| self.cookie(addr, salt, p) == cookie);
                if !fresh {
                    self.guard.bad_cookie += 1;
                    return;
                }
                // Адрес настоящий — теперь можно и отказать: бан, перебор пароля, пароль.
                if self.banned.contains(&addr.ip()) {
                    self.send_to(addr, &protocol::reject("вход на этот сервер вам закрыт"));
                    return;
                }
                if let Some(&(tries, last)) = self.wrong_passwords.get(&addr.ip())
                    && tries >= PASSWORD_TRIES
                    && last.elapsed() < PASSWORD_LOCKOUT
                {
                    self.send_to(addr, &protocol::reject("слишком много неверных паролей — попробуйте через минуту"));
                    return;
                }
                let password = self.identity.open_from(response.key, &response.challenge.aad(), &response.password);
                let Some(password) = password else {
                    self.guard.forged += 1;
                    return;
                };
                if !self.password.is_empty() && !same(&password, self.password.as_bytes()) {
                    self.guard.wrong_password += 1;
                    let entry = self.wrong_passwords.entry(addr.ip()).or_insert((0, Instant::now()));
                    *entry = (if entry.1.elapsed() < PASSWORD_LOCKOUT { entry.0 + 1 } else { 1 }, Instant::now());
                    self.send_to(addr, &protocol::reject("неверный пароль"));
                    return;
                }
                self.wrong_passwords.remove(&addr.ip());
                self.accept(response.hello, response.key, addr, world);
            }
            Kind::Input | Kind::Bye => {
                // Только свой клиент и только зашифрованное его ключом: иначе кто-то
                // подставил адрес, чтобы управлять чужим солдатом или выкинуть игрока.
                let Some(i) = index else { return };
                let Some(plain) = self.clients[i].session.open(packet) else {
                    self.guard.forged += 1;
                    return;
                };
                let data = &plain[protocol::HEADER..];
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
            Kind::Welcome | Kind::Snapshot | Kind::Reject | Kind::Challenge | Kind::Kick => {}
        }
    }

    fn accept(&mut self, hello: Hello, client_key: [u8; 32], addr: SocketAddr, world: &mut World) {
        // Своя пара ключей на каждое подключение; негодный ключ клиента — не пускаем.
        let keys = Keypair::generate();
        let Some(session) = Session::server(&keys, client_key) else {
            self.guard.forged += 1;
            return;
        };
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
        let welcome = Welcome { server_key: keys.public, server_name: self.name.clone(), ..welcome_for(world, id, soldier, &self.config) }.encode();
        let name = if hello.name.trim().is_empty() { format!("Игрок {id}") } else { hello.name };
        self.log.push(format!("{name} подключился ({addr}), солдат #{soldier}"));
        self.clients.push(RemoteClient {
            id,
            addr,
            name,
            team,
            replica: ServerReplica::new(soldier, world),
            traffic: Traffic::default(),
            session,
            last_seq: 0,
            last_heard: Instant::now(),
            welcome: welcome.clone(),
        });
        let i = self.clients.len() - 1;
        self.send(i, &welcome);
    }

    /// Выкинуть игрока с сервера: он узнает причину (пакет зашифрован — подделать
    /// такой нельзя), его солдат вернётся к ИИ. `false` — такого игрока нет.
    pub fn kick(&mut self, id: u16, reason: &str, world: &mut World) -> bool {
        let Some(i) = self.clients.iter().position(|c| c.id == id) else { return false };
        let packet = self.clients[i].session.seal(&protocol::kick(reason));
        // Дважды — пакет может потеряться.
        self.send(i, &packet);
        self.send(i, &packet);
        let c = self.clients.remove(i);
        world.release_control(c.replica.soldier);
        self.log.push(format!("{} выкинут: {reason}", c.name));
        true
    }

    /// Забанить игрока: его адрес больше не пустят, а сам он выкинут.
    /// Возвращает его адрес (чтобы записать в список банов).
    pub fn ban(&mut self, id: u16, reason: &str, world: &mut World) -> Option<SocketAddr> {
        let addr = self.clients.iter().find(|c| c.id == id)?.addr;
        if !self.banned.contains(&addr.ip()) {
            self.banned.push(addr.ip());
        }
        self.kick(id, &format!("бан: {reason}"), world);
        Some(addr)
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
        // Туман войны: клиенту — только тех врагов, кого видит его команда.
        if !self.clients.is_empty() {
            self.vision.update(world);
        }
        // Снимки клиентов независимы — собираем их на всех ядрах.
        let config = &self.config;
        let vision = &self.vision;
        let packets: Vec<Vec<u8>> = self
            .clients
            .par_iter_mut()
            .map(|c| c.replica.build_snapshot(world, config, board.as_ref(), Some(vision)))
            .collect();
        for (i, packet) in packets.iter().enumerate() {
            let sealed = self.clients[i].session.seal(packet);
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

/// Сравнение без раннего выхода: по времени ответа нельзя угадать, сколько первых
/// букв пароля совпало.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Приветствие для клиента, которому достался солдат `soldier`.
pub fn welcome_for(world: &World, client_id: u16, soldier: u32, config: &ReplicationConfig) -> Welcome {
    let c = &world.config;
    Welcome {
        server_key: [0; 32],
        server_name: String::new(),
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

    /// Полное подключение: привет → вызов → ответ → приветствие с ключом сервера.
    /// Возвращает ключи сессии клиента.
    /// Ответ на вызов с паролем, зашифрованным для ключа сервера.
    fn response_with(keys: &Keypair, challenge: Challenge, password: &str) -> Response {
        let sealed = keys.seal_for(challenge.server_key, &challenge.aad(), password.as_bytes()).unwrap();
        Response { challenge, hello: Hello { name: "Игрок".into(), team: Some(0) }, key: keys.public, password: sealed }
    }

    fn join(server: &mut NetServer, w: &mut World, sock: &UdpSocket, addr: SocketAddr) -> Session {
        join_with(server, w, sock, addr, "").expect("впустили")
    }

    /// Подключение с паролем; `None` — сервер отказал.
    fn join_with(server: &mut NetServer, w: &mut World, sock: &UdpSocket, addr: SocketAddr, password: &str) -> Option<Session> {
        let before = server.clients.len();
        let hello = protocol::hello(42);
        feed(server, w, &hello, addr);
        let reply = recv(sock).expect("вызов");
        assert!(reply.len() <= hello.len(), "ответ на привет не больше самого привета");
        let (kind, data) = protocol::parse(&reply).unwrap();
        assert_eq!(kind, Kind::Challenge);
        let challenge = Challenge::decode(data).unwrap();
        assert_eq!(challenge.salt, 42);
        assert_eq!(server.clients.len(), before, "на привет место не выделяем");
        let keys = Keypair::generate();
        let response = response_with(&keys, challenge, password);
        feed(server, w, &response.encode(), addr);
        let (kind, data) = protocol::parse(&recv(sock).expect("приветствие")).map(|(k, d)| (k, d.to_vec())).unwrap();
        if kind == Kind::Reject {
            return None;
        }
        assert_eq!(kind, Kind::Welcome);
        let welcome = Welcome::decode(&data).unwrap();
        assert_eq!(welcome.server_name, server.name);
        Some(Session::client(&keys, welcome.server_key).expect("ключ сервера годный"))
    }

    fn input(seq: u32, fire: bool) -> Vec<u8> {
        let p = InputPacket { seq, ack_tick: 0, time_ms: 1, input: PlayerInput { fire, aim: Vec2::ONE, ..Default::default() } };
        p.encode()
    }

    #[test]
    fn honest_client_joins_and_plays() {
        let (mut s, mut w) = (server(), world());
        let (sock, addr) = player();
        let mut session = join(&mut s, &mut w, &sock, addr);
        assert_eq!(s.clients.len(), 1);
        feed(&mut s, &mut w, &session.seal(&input(1, true)), addr);
        let soldier = s.clients[0].replica.soldier as usize;
        assert!(matches!(w.soldiers[soldier].controller, wind_sim::Controller::Player(i) if i.fire), "зашифрованный ввод принят");
        // Снимки приходят зашифрованными, и клиент их открывает.
        s.send_snapshots(&w);
        let snap = recv(&sock).unwrap();
        let (kind, _) = protocol::parse(&snap).unwrap();
        assert_eq!(kind, Kind::Snapshot);
        let plain = session.open(&snap).expect("снимок расшифровывается");
        assert_eq!(protocol::parse(&plain).unwrap().0, Kind::Snapshot);
        assert_eq!(s.guard, Guard::default());
    }

    #[test]
    fn forged_input_and_bye_are_ignored() {
        let (mut s, mut w) = (server(), world());
        let (sock, addr) = player();
        let mut session = join(&mut s, &mut w, &sock, addr);
        // Кто-то подставил адрес игрока, но его ключей не знает: шифрует своими или
        // не шифрует вовсе.
        let (_, attacker) = {
            let (c, srv) = (Keypair::generate(), Keypair::generate());
            (Session::server(&srv, c.public).unwrap(), Session::client(&c, srv.public).unwrap())
        };
        let mut attacker = attacker;
        feed(&mut s, &mut w, &attacker.seal(&input(5, true)), addr);
        feed(&mut s, &mut w, &input(6, true), addr);
        let soldier = s.clients[0].replica.soldier as usize;
        assert!(matches!(w.soldiers[soldier].controller, wind_sim::Controller::Player(i) if !i.fire), "чужой ввод не принят");
        feed(&mut s, &mut w, &attacker.seal(&protocol::bye()), addr);
        feed(&mut s, &mut w, &protocol::bye(), addr);
        assert_eq!(s.clients.len(), 1, "игрока не выкинули");
        assert_eq!(s.guard.forged, 4);
        // А свой выход — работает.
        feed(&mut s, &mut w, &session.seal(&protocol::bye()), addr);
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
        let response = response_with(&Keypair::generate(), challenge, "");
        feed(&mut s, &mut w, &response.encode(), victim);
        // Выдуманная печать — тоже нет.
        let fake = response_with(&Keypair::generate(), Challenge { salt: 7, cookie: 0xDEAD, server_key: challenge.server_key }, "");
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

    #[test]
    fn password_is_checked_and_guessing_is_locked_out() {
        let (mut s, mut w) = (server(), world());
        s.password = "секрет".into();
        let (sock, addr) = player();
        assert!(join_with(&mut s, &mut w, &sock, addr, "").is_none(), "без пароля — нет");
        assert!(join_with(&mut s, &mut w, &sock, addr, "Секрет").is_none(), "неверный — нет");
        assert!(join_with(&mut s, &mut w, &sock, addr, "секрет").is_some(), "верный — да");
        assert_eq!(s.clients.len(), 1);
        assert_eq!(s.guard.wrong_password, 2);
        // Перебор: после пяти неверных подряд даже верный пароль минуту не примут.
        let (sock2, addr2) = player();
        let _ = sock2.local_addr();
        for k in 0..PASSWORD_TRIES {
            assert!(join_with(&mut s, &mut w, &sock2, addr2, &format!("угадай{k}")).is_none());
        }
        // Тот же IP (127.0.0.1) — и первый игрок с него уже внутри, но новых не пустят.
        let (sock3, addr3) = player();
        assert!(join_with(&mut s, &mut w, &sock3, addr3, "секрет").is_none(), "IP на минуту закрыт");
    }

    #[test]
    fn kick_and_ban() {
        let (mut s, mut w) = (server(), world());
        let (sock, addr) = player();
        let session = join(&mut s, &mut w, &sock, addr);
        let id = s.clients[0].id;
        assert!(s.kick(id, "проверка", &mut w));
        assert!(s.clients.is_empty());
        // Игрок узнаёт причину — из зашифрованного пакета.
        let packet = recv(&sock).expect("пакет «выкинут»");
        let plain = session.open(&packet).expect("кик зашифрован ключом сессии");
        let (kind, data) = protocol::parse(&plain).unwrap();
        assert_eq!(kind, Kind::Kick);
        assert_eq!(protocol::decode_reject(data), "проверка");
        assert!(!s.kick(id, "ещё раз", &mut w), "второй раз выкидывать некого");
        // Бан: выкинут и больше не войдёт с этого адреса.
        while recv(&sock).is_some() {}
        let (sock2, addr2) = player();
        join(&mut s, &mut w, &sock2, addr2);
        let id2 = s.clients[0].id;
        assert_eq!(s.ban(id2, "читы", &mut w), Some(addr2));
        assert!(s.banned.contains(&addr2.ip()));
        while recv(&sock2).is_some() {}
        assert!(join_with(&mut s, &mut w, &sock2, addr2, "").is_none(), "забаненный не входит");
        s.banned.clear();
        assert!(join_with(&mut s, &mut w, &sock2, addr2, "").is_some(), "разбан — входит");
    }
}
