//! Настоящий клиент подключается к настоящему серверу по UDP: привет, вызов, ответ,
//! приветствие с токеном — и дальше снимки и ввод.

use std::time::{Duration, Instant};
use wind_net::{ClientStatus, NetClient, NetServer, ReplicationConfig};
use wind_sim::{PlayerInput, SimConfig, World};

#[test]
fn client_joins_over_udp_and_gets_snapshots() {
    let mut world = World::new(SimConfig { map_size: 128, soldiers_per_team: 6, capture_points: 1, ..Default::default() }, 3);
    let mut server = NetServer::bind("127.0.0.1:0", ReplicationConfig::default()).unwrap();
    let addr = server.local_addr().unwrap().to_string();
    let mut client = NetClient::connect(&addr, "Проверка", None, "").unwrap();
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(3) {
        client.poll();
        client.send_input(PlayerInput::default());
        server.poll(&mut world);
        world.step();
        server.send_snapshots(&world);
        if client.world.as_ref().is_some_and(|w| w.tick > 20) {
            break;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(client.status, ClientStatus::Connected);
    assert_eq!(server.clients.len(), 1);
    assert!(client.world.as_ref().unwrap().tick > 20, "снимки идут");
    assert_eq!(client.forged, 0);
    assert_eq!(server.guard, wind_net::server::Guard::default(), "у честного клиента ничего не отбито");
    // Выход — с токеном, сервер его принимает.
    client.disconnect();
    std::thread::sleep(Duration::from_millis(20));
    server.poll(&mut world);
    assert!(server.clients.is_empty());
}
