//! Управление своим солдатом по сети: как быстро он откликается на нажатие и
//! сходится ли с сервером. Без сокетов: пакеты идут в памяти с задержкой.

use std::collections::VecDeque;
use wind_net::pilot::Pilot;
use wind_net::protocol::{self, InputPacket};
use wind_net::replication::{RemoteWorld, ReplicationConfig, ServerReplica};
use wind_net::server::welcome_for;
use wind_sim::{CLAIM_CATCH_UP, PlayerInput, SimConfig, Vec2, World};

/// Задержка в одну сторону, с. Пинг — вдвое больше.
const LATENCY: f64 = 0.05;
const FRAME: f64 = 1.0 / 144.0;

struct Net {
    world: World,
    soldier: u32,
    replica: ServerReplica,
    client: RemoteWorld,
    config: ReplicationConfig,
    to_server: VecDeque<(f64, Vec<u8>)>,
    to_client: VecDeque<(f64, Vec<u8>)>,
    pilot: Pilot,
    now: f64,
    next_tick: f64,
    seq: u32,
}

impl Net {
    fn new() -> Self {
        let sim = SimConfig { map_size: 256, soldiers_per_team: 4, capture_points: 3, ..Default::default() };
        let mut world = World::new(sim, 3);
        let soldier = world.take_control(0).unwrap();
        let config = ReplicationConfig::default();
        let welcome = welcome_for(&world, 1, soldier, &config);
        Net {
            replica: ServerReplica::new(soldier, &world),
            client: RemoteWorld::new(welcome, std::path::Path::new(".")).unwrap(),
            world,
            soldier,
            config,
            to_server: VecDeque::new(),
            to_client: VecDeque::new(),
            pilot: Pilot::default(),
            now: 0.0,
            next_tick: 0.0,
            seq: 0,
        }
    }

    fn server_pos(&self) -> Vec2 {
        self.world.soldiers[self.soldier as usize].pos
    }

    /// Один кадр клиента. Возвращает, где клиент рисует своего солдата.
    fn frame(&mut self, move_dir: Vec2, predict: bool, cheat: Option<Vec2>) -> Option<Vec2> {
        self.now += FRAME;
        // Сервер: принимает дошедший ввод и тикает 20 раз в секунду.
        while self.to_server.front().is_some_and(|p| p.0 <= self.now) {
            let (_, packet) = self.to_server.pop_front().unwrap();
            let (_, data) = protocol::parse(&packet).unwrap();
            let input = InputPacket::decode(data).unwrap();
            self.replica.on_ack(input.ack_tick);
            self.world.set_input(self.soldier, input.input);
        }
        while self.next_tick <= self.now {
            self.next_tick += 0.05;
            self.world.step();
            let snapshot = self.replica.build_snapshot(&self.world, &self.config);
            self.to_client.push_back((self.now + LATENCY, snapshot));
        }
        // Клиент: разбирает дошедшие снимки, двигает своего солдата, шлёт ввод.
        while self.to_client.front().is_some_and(|p| p.0 <= self.now) {
            let (_, packet) = self.to_client.pop_front().unwrap();
            let (_, data) = protocol::parse(&packet).unwrap();
            self.client.apply_snapshot(data, self.now).unwrap();
        }
        let w = &self.client;
        let server = w.my_soldier().filter(|_| w.me.alive).map(|s| s.pos);
        let (speed, radius) = (w.welcome.soldier_speed, w.welcome.soldier_radius);
        let shown = if predict {
            self.pilot.update(&w.map, server, move_dir, speed, radius, FRAME as f32)
        } else {
            w.my_soldier().map(|s| w.render_pos(s, self.now))
        };
        let claim = if predict { cheat.or(shown) } else { None };
        self.seq += 1;
        let input = PlayerInput { move_dir, aim: shown.unwrap_or_default() + Vec2::X, fire: false, grenade: false, pos: claim };
        let packet = InputPacket { seq: self.seq, ack_tick: w.tick, time_ms: 1, input }.encode();
        self.to_server.push_back((self.now + LATENCY, packet));
        shown
    }
}

/// Сколько секунд от нажатия до того, как свой солдат сдвинулся на экране.
fn response_time(predict: bool) -> f64 {
    let mut net = Net::new();
    // Подключились и постояли — снимки дошли, солдат виден.
    let mut start = None;
    for _ in 0..144 {
        start = net.frame(Vec2::ZERO, predict, None);
    }
    let start = start.expect("свой солдат должен быть виден");
    let pressed = net.now;
    for _ in 0..144 {
        let shown = net.frame(Vec2::X, predict, None).unwrap();
        if shown.distance(start) > 0.02 {
            return net.now - pressed;
        }
    }
    f64::INFINITY
}

#[test]
fn own_soldier_responds_at_once() {
    let old = response_time(false);
    let new = response_time(true);
    println!("пинг {:.0} мс: солдат сдвинулся через {:.0} мс по-старому и через {:.0} мс с предсказанием", LATENCY * 2000.0, old * 1000.0, new * 1000.0);
    assert!(old > LATENCY * 2.0, "по-старому отклик не может быть быстрее пинга: {old}");
    assert!(new <= FRAME * 1.5, "с предсказанием солдат должен двинуться в первом же кадре: {new}");
}

#[test]
fn server_follows_the_client() {
    let mut net = Net::new();
    for _ in 0..144 {
        net.frame(Vec2::ZERO, true, None);
    }
    // Бежим по кругу 3 секунды: сервер отстаёт не больше чем на пинг.
    let mut worst = 0.0f32;
    for i in 0..432 {
        let a = i as f32 / 144.0 * 2.0;
        let shown = net.frame(Vec2::new(a.cos(), a.sin()), true, None).unwrap();
        worst = worst.max(shown.distance(net.server_pos()));
    }
    // Встали — через секунду сервер там же, где клиент.
    for _ in 0..144 {
        net.frame(Vec2::ZERO, true, None);
    }
    let gap = net.pilot.pos().unwrap().distance(net.server_pos());
    println!("на бегу сервер отстаёт до {worst:.2} м, после остановки — {gap:.3} м, возвратов {}", net.pilot.snaps);
    assert_eq!(net.pilot.snaps, 0, "клиента не должно было возвращать");
    assert!(worst < 1.5, "сервер отстаёт слишком сильно: {worst}");
    assert!(gap < 0.05, "после остановки клиент и сервер разошлись: {gap}");
}

#[test]
fn server_rejects_teleport() {
    let mut net = Net::new();
    for _ in 0..144 {
        net.frame(Vec2::ZERO, true, None);
    }
    let before = net.server_pos();
    // Подделка: «я уже на 10 м восточнее» — сервер не слушает, клиент возвращается.
    let fake = before + Vec2::new(10.0, 0.0);
    for _ in 0..72 {
        net.frame(Vec2::ZERO, true, Some(fake));
    }
    assert!(net.server_pos().distance(before) < 0.01, "сервер поверил в телепорт");
    // Подделка поскромнее: «я на 1,5 м восточнее» — сервер идёт туда шагом, не прыжком.
    let near = before + Vec2::new(1.5, 0.0);
    let speed = net.world.config.soldier.speed;
    net.frame(Vec2::ZERO, true, Some(near));
    let mut last = net.server_pos();
    for _ in 0..72 {
        net.frame(Vec2::ZERO, true, Some(near));
        let now = net.server_pos();
        assert!(now.distance(last) <= speed * CLAIM_CATCH_UP * 0.05 + 1e-3, "сервер перескочил");
        last = now;
    }
}
