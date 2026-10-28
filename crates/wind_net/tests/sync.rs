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
                ..Default::default()
            },
        );
        self.world.step();
        let packet = self.replica.build_snapshot(&self.world, &self.config, None, None);
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
            // Гарантированно ломаем стены, даже если гранат не было: у восьми настоящих
            // разрушаемых стен (где они — зависит от генератора карт).
            let map = &link.world.map;
            let walls: Vec<Vec2> = (0..map.solid.len())
                .filter(|&c| map.solid[c] != 0 && map.solid[c] != wind_sim::map::INDESTRUCTIBLE)
                .step_by(97)
                .take(8)
                .map(|c| Vec2::new((c as u32 % map.width) as f32 + 0.5, (c as u32 / map.width) as f32 + 0.5))
                .collect();
            assert_eq!(walls.len(), 8);
            for at in walls {
                link.world.map.damage_circle(at, 6.0, 250.0);
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

#[test]
fn round_state_and_restored_map_reach_client() {
    let config = ReplicationConfig { packet_budget: 64 * 1024, ..Default::default() };
    let mut link = Link::new(30, config);
    link.world.config.round.time_limit = 2.0;
    link.world.config.round.intermission = 1.0;
    // Ломаем стены в первом раунде.
    let walls: Vec<Vec2> = (0..link.world.map.solid.len())
        .filter(|&i| link.world.map.solid[i] != 0 && link.world.map.solid[i] != wind_sim::map::INDESTRUCTIBLE)
        .step_by(50)
        .take(8)
        .map(|i| Vec2::new((i as u32 % link.world.map.width) as f32 + 0.5, (i as u32 / link.world.map.width) as f32 + 0.5))
        .collect();
    for at in walls {
        link.world.map.damage_circle(at, 3.0, 250.0);
    }
    // Раунд — 2 с (40 тиков): через 45 тиков идут итоги.
    for _ in 0..45 {
        link.tick(0.0);
    }
    assert!(!link.client.round.playing(), "у клиента должны быть итоги раунда");
    assert_eq!(link.client.round.phase, link.world.round.phase);
    // Перерыв (1 с) кончился — новый раунд, стены целые и у сервера, и у клиента.
    for _ in 0..30 {
        link.tick(0.0);
    }
    assert_eq!(link.client.round.number, 2);
    assert!(link.client.round.playing());
    assert!(link.client.map.solid == link.world.map.solid, "клиент должен получить восстановленные стены");
}


#[test]
fn board_with_player_names_reaches_client() {
    use wind_net::replication::{board, write_board};
    let config = ReplicationConfig { packet_budget: 64 * 1024, ..Default::default() };
    let mut link = Link::new(40, config);
    for _ in 0..200 {
        link.tick(0.0);
    }
    // Сервер рассылает таблицу раз в секунду — здесь шлём её вручную.
    let names = vec![(link.soldier, "Артём".to_string())];
    let rows = board(&link.world, &names);
    let mut w = wind_net::bits::BitWriter::new();
    write_board(&mut w, &rows);
    link.world.step();
    let packet = link.replica.build_snapshot(&link.world, &link.config, Some(&w), None);
    let (_, data) = protocol::parse(&packet).unwrap();
    link.client.apply_snapshot(data, 100.0).unwrap();
    assert_eq!(link.client.board, rows);
    assert_eq!(link.client.name_of(link.soldier), "Артём");
    assert!(rows.iter().any(|r| r.soldier == link.soldier), "игрок всегда в таблице");
}

#[test]
fn own_shots_are_heard_where_the_shooter_is() {
    use wind_net::replication::Heard;
    let mut link = Link::new(10, ReplicationConfig::default());
    for _ in 0..40 {
        link.tick(0.0);
    }
    let me = link.client.my_soldier().unwrap().pos;
    let shots: Vec<Vec2> = link
        .client
        .heard
        .iter()
        .filter_map(|h| match *h {
            Heard::Shot { pos, shooter, .. } if shooter == link.soldier => Some(pos),
            _ => None,
        })
        .collect();
    assert!(!shots.is_empty(), "игрок стрелял каждый тик — выстрелы должны быть слышны");
    // Солдат бегает по кругу: последний выстрел — недалеко от того, где он сейчас.
    assert!(shots.last().unwrap().distance(me) < 2.0);
}

#[test]
fn prone_soldier_is_prone_on_client() {
    let config = ReplicationConfig { packet_budget: 64 * 1024, ..Default::default() };
    let mut link = Link::new(10, config);
    link.tick(0.0);
    // Ближайший к игроку солдат ИИ прижат огнём — залегает.
    let me = link.world.soldiers[link.soldier as usize].pos;
    let near = (0..link.world.soldiers.len())
        .filter(|&i| i as u32 != link.soldier)
        .min_by(|&a, &b| link.world.soldiers[a].pos.distance(me).total_cmp(&link.world.soldiers[b].pos.distance(me)))
        .unwrap();
    for _ in 0..3 {
        link.world.soldiers[near].suppression = 1.0;
        link.tick(0.0);
    }
    assert!(link.world.soldiers[near].prone);
    let seen = link.client.soldiers[near].as_ref().expect("клиент видит соседа");
    assert!(seen.net.is_prone(), "клиент знает, что он лежит");
}


#[test]
fn client_never_learns_about_hidden_enemies() {
    use wind_net::replication::Heard;
    use wind_net::vision::Vision;
    let config = ReplicationConfig { packet_budget: 64 * 1024, ..Default::default() };
    // Обзор короткий: иначе на карте 256 почти каждого врага кто-то из своих видит,
    // и проверять было бы нечего.
    let mut link = Link::new(40, config);
    link.world.config.soldier.vision_range = 20.0;
    let mut vision = Vision::default();
    let team = link.world.soldiers[link.soldier as usize].team;
    let (mut hidden_in_range, mut hidden_shots) = (0, 0);
    for _ in 0..900 {
        // Игрок идёт к середине карты — туда, где бой.
        let me = link.world.soldiers[link.soldier as usize].pos;
        let input = PlayerInput { move_dir: (Vec2::splat(128.0) - me).normalize_or_zero(), aim: me + Vec2::X, ..Default::default() };
        link.world.set_input(link.soldier, input);
        link.world.step();
        vision.update(&link.world);
        let packet = link.replica.build_snapshot(&link.world, &link.config, None, Some(&vision));
        let (_, data) = protocol::parse(&packet).unwrap();
        let now = link.world.tick as f64 * 0.05;
        link.client.apply_snapshot(data, now).unwrap();
        link.replica.on_ack(link.client.tick);
        // Каждый враг, о котором знает клиент, — виден его команде.
        for (id, s) in link.client.soldiers.iter().enumerate() {
            if s.as_ref().is_some_and(|s| s.net.team() != team) {
                assert!(vision.sees(&link.world, team, id as u32), "клиент знает о скрытом враге #{id}");
            }
        }
        let me = link.world.soldiers[link.soldier as usize].pos;
        hidden_in_range += link
            .world
            .soldiers
            .iter()
            .enumerate()
            .filter(|(i, s)| s.alive && s.team != team && s.pos.distance(me) < link.config.interest_radius && !vision.sees(&link.world, team, *i as u32))
            .count();
        // Выстрелы тех, кого не видно, всё равно слышны — с места стрелка.
        for h in link.client.heard.drain(..) {
            if let Heard::Shot { shooter, .. } = h
                && link.client.soldiers[shooter as usize].is_none()
            {
                hidden_shots += 1;
            }
        }
    }
    println!("скрытых врагов в радиусе интереса (сумма по тикам): {hidden_in_range}, выстрелов скрытых стрелков: {hidden_shots}");
    assert!(hidden_in_range > 100, "проверка что-то значит: враги в радиусе бывают скрыты");
    assert!(hidden_shots > 0, "выстрелы из-за стен слышны");
}
