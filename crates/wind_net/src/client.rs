//! Клиент: подключается к серверу, шлёт ввод, собирает мир из снимков.

use crate::protocol::{self, Hello, InputPacket, Kind, Welcome};
use crate::replication::{RemoteWorld, SnapshotError};
use crate::traffic::Traffic;
use std::io::{self, ErrorKind};
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::{Duration, Instant};
use wind_sim::PlayerInput;
use wind_sim::rng::Rng;

const HELLO_INTERVAL: Duration = Duration::from_millis(250);
const TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq)]
pub enum ClientStatus {
    Connecting,
    Connected,
    Rejected(String),
    TimedOut,
}

pub struct NetClient {
    socket: UdpSocket,
    server: SocketAddr,
    hello: Hello,
    pub status: ClientStatus,
    pub world: Option<RemoteWorld>,
    pub traffic: Traffic,
    /// Время туда-обратно, мс.
    pub rtt_ms: f32,
    /// Доля входящих пакетов, которые выбрасываем нарочно, — проверить, как сеть
    /// переживает потери. 0 — ничего не теряем.
    pub simulated_loss: f32,
    /// Снимки, которые не удалось разобрать (нет базы или битые).
    pub decode_errors: u32,
    started: Instant,
    last_hello: Option<Instant>,
    last_heard: Instant,
    seq: u32,
    rng: Rng,
    buf: Vec<u8>,
}

impl NetClient {
    pub fn connect(server: &str, name: &str, team: Option<u8>) -> io::Result<Self> {
        let server = server
            .to_socket_addrs()?
            .find(|a| a.is_ipv4())
            .ok_or_else(|| io::Error::new(ErrorKind::NotFound, "адрес сервера не найден"))?;
        let socket = UdpSocket::bind("0.0.0.0:0")?;
        socket.set_nonblocking(true)?;
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| d.as_nanos() as u64);
        Ok(NetClient {
            socket,
            server,
            hello: Hello { name: name.to_owned(), team },
            status: ClientStatus::Connecting,
            world: None,
            traffic: Traffic::default(),
            rtt_ms: 0.0,
            simulated_loss: 0.0,
            decode_errors: 0,
            started: Instant::now(),
            last_hello: None,
            last_heard: Instant::now(),
            seq: 0,
            rng: Rng::new(seed),
            buf: vec![0; protocol::MAX_PACKET],
        })
    }

    /// Секунды с момента создания клиента — часы для интерполяции.
    pub fn now(&self) -> f64 {
        self.started.elapsed().as_secs_f64()
    }

    pub fn server_addr(&self) -> SocketAddr {
        self.server
    }

    pub fn poll(&mut self) {
        if self.status == ClientStatus::Connecting
            && self.last_hello.is_none_or(|t| t.elapsed() > HELLO_INTERVAL)
        {
            self.last_hello = Some(Instant::now());
            let hello = self.hello.encode();
            self.send(&hello);
        }
        loop {
            match self.socket.recv_from(&mut self.buf) {
                Ok((n, from)) => {
                    if from != self.server {
                        continue;
                    }
                    if self.simulated_loss > 0.0 && self.rng.chance(self.simulated_loss) {
                        continue;
                    }
                    self.traffic.on_recv(n);
                    self.last_heard = Instant::now();
                    let packet = self.buf[..n].to_vec();
                    self.handle(&packet);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::ConnectionReset => continue,
                Err(_) => break,
            }
        }
        if matches!(self.status, ClientStatus::Connecting | ClientStatus::Connected)
            && self.last_heard.elapsed() > TIMEOUT
        {
            self.status = ClientStatus::TimedOut;
        }
        self.traffic.roll();
    }

    fn handle(&mut self, packet: &[u8]) {
        let Some((kind, data)) = protocol::parse(packet) else { return };
        match kind {
            Kind::Welcome => {
                if self.world.is_none()
                    && let Ok(welcome) = Welcome::decode(data)
                {
                    self.world = Some(RemoteWorld::new(welcome));
                    self.status = ClientStatus::Connected;
                }
            }
            Kind::Snapshot => {
                let now = self.now();
                let now_ms = (now * 1000.0) as u32;
                let Some(world) = &mut self.world else { return };
                match world.apply_snapshot(data, now) {
                    Ok(()) => {
                        if world.echo_time > 0 {
                            let rtt = now_ms.saturating_sub(world.echo_time) as f32;
                            self.rtt_ms = if self.rtt_ms == 0.0 { rtt } else { self.rtt_ms * 0.9 + rtt * 0.1 };
                        }
                    }
                    Err(SnapshotError::MissingBaseline) | Err(SnapshotError::Truncated) => {
                        self.decode_errors += 1;
                    }
                }
            }
            Kind::Reject => self.status = ClientStatus::Rejected(protocol::decode_reject(data)),
            Kind::Hello | Kind::Input | Kind::Bye => {}
        }
    }

    /// Шлёт ввод вместе с подтверждением последнего снимка.
    pub fn send_input(&mut self, input: PlayerInput) {
        let Some(world) = &self.world else { return };
        if self.status != ClientStatus::Connected {
            return;
        }
        self.seq += 1;
        let packet = InputPacket {
            seq: self.seq,
            ack_tick: world.tick,
            time_ms: ((self.now() * 1000.0) as u32).max(1),
            input,
        }
        .encode();
        self.send(&packet);
    }

    pub fn disconnect(&mut self) {
        if self.status == ClientStatus::Connected {
            let bye = protocol::bye();
            self.send(&bye);
            self.send(&bye);
        }
        self.status = ClientStatus::TimedOut;
    }

    fn send(&mut self, packet: &[u8]) {
        self.traffic.on_send(packet.len());
        let _ = self.socket.send_to(packet, self.server);
    }
}

impl Drop for NetClient {
    fn drop(&mut self) {
        self.disconnect();
    }
}
