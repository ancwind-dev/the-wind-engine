//! Сходится ли мир клиента с сервером. Без сокетов: пакеты передаются
//! в памяти, а «сеть» теряет их с заданной вероятностью.

use wind_net::protocol;
use wind_net::replication::{NetSoldier, RemoteWorld, ReplicationConfig, ServerReplica};
use wind_net::server::welcome_for;
use wind_sim::rng::Rng;
use wind_sim::{PlayerInput, SimConfig, Vec2, World};

struct Link {
    world: World,
    soldier: u32,
    replica: ServerReplica,
    client: RemoteWorld,
    config: ReplicationConfig,
    rng: Rng,
    sizes: Vec<usize>,
    errors: u32,
}

impl Link {
    fn new(per_team: u32, config: ReplicationConfig) -> Self {
        let sim = SimConfig { map_size: 256, soldiers_per_team: per_team, capture_points: 3, ..Default::default() };
        let mut world = World::new(sim, 7);
        let soldier = world.take_control(0).unwrap();
        let welcome = welcome_for(&world, 1, soldier, &config);
        Link {
            replica: ServerReplica::new(soldier, &world),
            client: RemoteWorld::new(welcome, std::path::Path::new(".")).unwrap(),
            world,
            soldier,
            config,
            rng: Rng::new(99),
            sizes: Vec::new(),
            errors: 0,
        }
    }

    /// Один тик: игрок бегает по кругу и стреляет, сервер шлёт снимок, клиент подтверждает.
    fn tick(&mut self, loss: f32) {
        let t = self.world.tick as f32 * 0.05;
        let me = self.world.soldiers[self.soldier as usize].pos;
        self.world.set_input(
            self.soldier,
            PlayerInput {
                move_dir: Vec2::new(t.cos(), t.sin()),
                aim: me + Vec2::new(10.0, 0.0),
                fire: true,
                grenade: false,
            },
        );
        self.world.step();
        let packet = self.replica.build_snapshot(&self.world, &self.config);
        self.sizes.push(packet.len());
        if !self.rng.chance(loss) {
            let (_, data) = protocol::parse(&packet).unwrap();
            let now = self.world.tick as f64 * 0.05;
            if self.client.apply_snapshot(data, now).is_err() {
                self.errors += 1;
            }
        }
        if !self.rng.chance(loss) {
            self.replica.on_ack(self.client.tick);
        }
    }

    /// Что сервер считает видимым клиенту прямо сейчас.
    fn server_view(&self) -> Vec<(u32, NetSoldier)> {
        let center = self.world.soldiers[self.soldier as usize].pos;
        let r2 = self.config.interest_radius.powi(2);
        self.world
            .soldiers
            .iter()
            .enumerate()
            .filter(|(_, s)| s.alive && s.pos.distance_squared(center) < r2)
            .map(|(i, s)| (i as u32, NetSoldier::from_soldier(s)))
            .collect()
    }

    fn client_view(&self) -> Vec<(u32, NetSoldier)> {
        self.client
            .soldiers
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.as_ref().map(|s| (i as u32, s.net)))
            .collect()
    }
}

#[test]
fn client_converges_under_loss() {
    let config = ReplicationConfig { packet_budget: 64 * 1024, ..Default::default() };
    let mut link = Link::new(150, config);
    for i in 0..600 {
        if i == 300 {
            // Гарантированно ломаем стены, даже если гранат не было.
            let center = Vec2::splat(128.0);
            for k in 0..8 {
                link.world.map.damage_circle(center + Vec2::new(k as f32 * 12.0 - 48.0, 0.0), 6.0, 250.0);
            }
        }
        link.tick(0.2);
    }
    // Связь наладилась — даём догнать.
    for _ in 0..30 {
        link.tick(0.0);
    }

    assert!(link.world.map.revision > 0, "карта должна была измениться");
    assert_eq!(link.errors, 0, "все дошедшие снимки должны разбираться");
    assert_eq!(link.client_view(), link.server_view(), "клиент видит не то, что сервер");
    assert!(link.client.map.solid == link.world.map.solid, "карта клиента не совпала с сервером");
    let me = &link.world.soldiers[link.soldier as usize];
    assert_eq!(link.client.me.kills, me.kills);
    assert_eq!(link.client.me.ammo, me.ammo);
}

#[test]
fn tight_budget_keeps_everyone_fresh() {
    let mut link = Link::new(300, ReplicationConfig::default());
    for _ in 0..400 {
        link.tick(0.0);
    }
    let server = link.server_view();
    let client = link.client_view();
    let mut missing = 0;
    let mut worst = 0.0f32;
    for (id, s) in &server {
        match client.binary_search_by_key(id, |e| e.0) {
            Ok(i) => worst = worst.max(client[i].1.pos().distance(s.pos())),
            Err(_) => missing += 1,
        }
    }
    let avg = link.sizes.iter().sum::<usize>() as f32 / link.sizes.len() as f32;
    let max = *link.sizes.iter().max().unwrap();
    println!(
        "видно солдат: {}, нет у клиента: {missing}, худшее отставание: {worst:.2} м, \
         пакет: средний {avg:.0} Б, максимум {max} Б, {:.1} КБ/с при 20 Гц",
        server.len(),
        avg * 20.0 / 1024.0
    );
    assert!(server.len() > 50, "в тесте должно быть тесно");
    assert!(missing * 20 <= server.len(), "слишком многие солдаты не доехали до клиента");
    assert!(worst < 3.0, "кто-то сильно отстал: {worst} м");
    assert!(max <= 1200 + 1100, "пакет вышел далеко за бюджет: {max}");
}

#[test]
fn file_map_is_loaded_and_checked() {
    use wind_sim::mapfile::MapFile;
    let dir = std::env::temp_dir().join(format!("wind_net_maps_{}", std::process::id()));
    let mut file = MapFile::generated("испытание", 128, 2, 9);
    file.save(&dir.join("испытание")).unwrap();

    let mut world = World::from_file(SimConfig { soldiers_per_team: 10, ..Default::default() }, 1, file.clone());
    let soldier = world.take_control(0).unwrap();
    let welcome = welcome_for(&world, 1, soldier, &ReplicationConfig::default());
    assert_eq!(welcome.map_name, "испытание");
    let client = RemoteWorld::new(welcome.clone(), &dir).expect("карта должна найтись");
    assert!(client.map.solid == world.map.solid);

    // У клиента карта другая — отказ с понятной причиной.
    file.map.solid[300] = 77;
    file.save(&dir.join("испытание")).unwrap();
    let err = RemoteWorld::new(welcome.clone(), &dir).err().unwrap();
    assert!(err.contains("не такая"), "{err}");
    // А карты нет совсем.
    let missing = RemoteWorld::new(welcome, &dir.join("нет")).err().unwrap();
    assert!(missing.contains("нет карты"), "{missing}");
    std::fs::remove_dir_all(&dir).ok();
}
