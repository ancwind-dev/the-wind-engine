//! Сервер: принимает клиентов по UDP, раздаёт им солдат, шлёт снимки.

use crate::protocol::{self, Hello, InputPacket, Kind, Welcome};
use crate::replication::{ReplicationConfig, ServerReplica};
use crate::traffic::Traffic;
use rayon::prelude::*;
use std::io::{self, ErrorKind};
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};
use wind_sim::World;

/// Клиент, от которого столько не было пакетов, считается отключившимся.
const TIMEOUT: Duration = Duration::from_secs(5);

pub struct RemoteClient {
    pub id: u16,
    pub addr: SocketAddr,
    pub name: String,
    pub team: u8,
    pub replica: ServerReplica,
    pub traffic: Traffic,
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

    fn handle(&mut self, packet: &[u8], addr: SocketAddr, world: &mut World) {
        let Some((kind, data)) = protocol::parse(packet) else { return };
        let index = self.clients.iter().position(|c| c.addr == addr);
        if let Some(i) = index {
            self.clients[i].last_heard = Instant::now();
            self.clients[i].traffic.on_recv(packet.len());
        }
        match kind {
            Kind::Hello => {
                if let Some(i) = index {
                    // Наш ответ потерялся — повторяем.
                    let welcome = self.clients[i].welcome.clone();
                    self.send(i, &welcome);
                    return;
                }
                let Ok(hello) = Hello::decode(data) else { return };
                self.accept(hello, addr, world);
            }
            Kind::Input => {
                let (Some(i), Ok(input)) = (index, InputPacket::decode(data)) else { return };
                let c = &mut self.clients[i];
                c.replica.on_ack(input.ack_tick);
                if input.seq > c.last_seq {
                    c.last_seq = input.seq;
                    c.replica.echo_time = input.time_ms;
                    world.set_input(c.replica.soldier, input.input);
                }
            }
            Kind::Bye => {
                if let Some(i) = index {
                    let c = self.clients.remove(i);
                    world.release_control(c.replica.soldier);
                    self.log.push(format!("{} отключился", c.name));
                }
            }
            Kind::Welcome | Kind::Snapshot | Kind::Reject => {}
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
        let welcome = welcome_for(world, id, soldier, &self.config).encode();
        let name = if hello.name.trim().is_empty() { format!("Игрок {id}") } else { hello.name };
        self.log.push(format!("{name} подключился ({addr}), солдат #{soldier}"));
        self.clients.push(RemoteClient {
            id,
            addr,
            name,
            team,
            replica: ServerReplica::new(soldier, world),
            traffic: Traffic::default(),
            last_seq: 0,
            last_heard: Instant::now(),
            welcome: welcome.clone(),
        });
        let i = self.clients.len() - 1;
        self.send(i, &welcome);
    }

    /// Шлёт каждому клиенту его снимок. Вызывать после тика.
    pub fn send_snapshots(&mut self, world: &World) {
        // Снимки клиентов независимы — собираем их на всех ядрах.
        let config = &self.config;
        let packets: Vec<Vec<u8>> = self
            .clients
            .par_iter_mut()
            .map(|c| c.replica.build_snapshot(world, config))
            .collect();
        for (i, packet) in packets.iter().enumerate() {
            self.send(i, packet);
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
        map_name: world.map_file.as_ref().map_or(String::new(), |f| f.0.clone()),
        map_fingerprint: world.map_file.as_ref().map_or(0, |f| f.1),
    }
}
