//! Выделенный сервер.
//!
//! cargo run --release -p wind_server                    — сервер на порту 7777
//! cargo run --release -p wind_server -- --name "Сервер Артёма" --password тайна
//! cargo run --release -p wind_server -- --bench         — замер симуляции без сети
//!
//! Пока сервер работает, в консоль можно писать команды: игроки, кик, бан, пароль…
//! («помощь» — список).

mod admin;

use std::path::PathBuf;
use std::time::{Duration, Instant};
use wind_net::{DEFAULT_PORT, NetServer, ReplicationConfig};
use wind_sim::mapfile::MapFile;
use wind_sim::{SimConfig, TickStats, World};

struct Args {
    config: PathBuf,
    soldiers: Option<u32>,
    map: Option<u32>,
    ticks: u64,
    seed: u64,
    host: String,
    port: u16,
    bench: bool,
    /// Карта из `data/maps/<имя>`. Без неё — процедурная.
    map_file: Option<String>,
    name: String,
    password: String,
    max_players: usize,
    /// Файл банов.
    bans: PathBuf,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        config: PathBuf::from("data/sim.ron"),
        soldiers: None,
        map: None,
        ticks: 2000,
        seed: 1,
        host: "0.0.0.0".into(),
        port: DEFAULT_PORT,
        bench: false,
        map_file: None,
        name: "The Wind Engine".into(),
        password: String::new(),
        max_players: 24,
        bans: PathBuf::from("data/bans.txt"),
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().ok_or(format!("после {flag} нужно значение"));
        match flag.as_str() {
            "--config" => args.config = PathBuf::from(value()?),
            "--soldiers" => args.soldiers = Some(number(&value()?)?),
            "--map" => args.map = Some(number(&value()?)?),
            "--ticks" => args.ticks = number(&value()?)?,
            "--seed" => args.seed = number(&value()?)?,
            "--host" => args.host = value()?,
            "--port" => args.port = number(&value()?)?,
            "--bench" => args.bench = true,
            "--map-file" => args.map_file = Some(value()?),
            "--name" => args.name = value()?,
            "--password" => args.password = value()?,
            "--max-players" => args.max_players = number(&value()?)?,
            "--bans" => args.bans = PathBuf::from(value()?),
            "--help" | "-h" => {
                println!(
                    "wind_server [--host 0.0.0.0] [--port 7777] [--config data/sim.ron] [--soldiers N на команду] \
                     [--map сторона, м] [--map-file имя из data/maps] [--seed N]\n\
                     \x20           [--name имя сервера] [--password пароль] [--max-players 24] [--bans data/bans.txt]\n\
                     wind_server --bench [--ticks N] ...   — замер симуляции без сети\n\
                     Пока сервер работает, команды в консоли: игроки, кик, бан, разбан, баны, пароль, имя, стоп («помощь»)."
                );
                std::process::exit(0);
            }
            other => return Err(format!("неизвестный флаг {other}, см. --help")),
        }
    }
    Ok(args)
}

fn number<T: std::str::FromStr>(s: &str) -> Result<T, String> {
    s.parse().map_err(|_| format!("не число: {s}"))
}

fn main() {
    let args = parse_args().unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    });
    let mut config = if args.config.exists() {
        SimConfig::load(&args.config).unwrap_or_else(|e| {
            eprintln!("{e}");
            std::process::exit(2);
        })
    } else {
        eprintln!("{} не найден, беру параметры по умолчанию", args.config.display());
        SimConfig::default()
    };
    if let Some(n) = args.soldiers {
        config.soldiers_per_team = n;
    }
    if let Some(m) = args.map {
        config.map_size = m;
    }
    if args.bench {
        bench(config, &args);
    } else {
        serve(config, &args);
    }
}

fn serve(config: SimConfig, args: &Args) {
    let mut world = make_world(config, args);
    let mut net = NetServer::bind((args.host.as_str(), args.port), ReplicationConfig::default()).unwrap_or_else(|e| {
        eprintln!("не удалось открыть порт {}: {e}", args.port);
        std::process::exit(1);
    });
    net.name = args.name.chars().take(wind_net::protocol::MAX_SERVER_NAME).collect();
    net.password = args.password.clone();
    net.max_clients = args.max_players.max(1);
    let mut bans = admin::Bans::load(&args.bans).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    });
    net.banned = bans.ips();
    // Консоль хозяина: строки читаем в отдельном потоке, чтобы тик не ждал ввода.
    let (lines_tx, lines) = std::sync::mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lines().map_while(Result::ok) {
            if lines_tx.send(line).is_err() {
                break;
            }
        }
    });
    println!(
        "Сервер «{}» слушает порт {}. Карта {2}×{2} м, {3} солдат, {4} Гц, игроков до {5}{6}. Банов: {7}.",
        net.name,
        args.port,
        world.config.map_size,
        world.soldiers.len(),
        world.config.tick_rate,
        net.max_clients,
        if net.password.is_empty() { ", без пароля" } else { ", с паролем" },
        bans.list.len()
    );
    println!("Команды — в консоль («помощь»). Остановить — «стоп» или Ctrl+C.");

    let dt = Duration::from_secs_f32(world.dt());
    let mut next = Instant::now();
    let mut last_report = Instant::now();
    let mut worst_ms = 0.0f32;
    // Сколько чужих пакетов отбито — пишем, когда число меняется.
    let mut guard_seen = net.guard;
    loop {
        let started = Instant::now();
        net.poll(&mut world);
        world.step();
        net.send_snapshots(&world);
        worst_ms = worst_ms.max(started.elapsed().as_secs_f32() * 1000.0);
        for line in net.log.drain(..) {
            println!("  {line}");
        }
        while let Ok(line) = lines.try_recv() {
            let out = admin::command(&line, &mut net, &mut world, &mut bans);
            for l in &out.lines {
                println!("{l}");
            }
            for l in net.log.drain(..) {
                println!("  {l}");
            }
            if out.quit {
                // Игроки узнают, что сервер закрылся, а не ждут таймаута.
                let ids: Vec<u16> = net.clients.iter().map(|c| c.id).collect();
                for id in ids {
                    net.kick(id, "сервер остановлен", &mut world);
                }
                return;
            }
        }

        if last_report.elapsed() >= Duration::from_secs(2) {
            last_report = Instant::now();
            if net.clients.is_empty() {
                println!("[тик {}] клиентов нет | тик сим {:.2} мс", world.tick, world.stats.total_ms);
            } else {
                let n = net.clients.len() as f32;
                let down = net.clients.iter().map(|c| c.traffic.sent_per_sec).sum::<f32>() / n;
                let up = net.clients.iter().map(|c| c.traffic.recv_per_sec).sum::<f32>() / n;
                let max_packet = net.clients.iter().map(|c| c.traffic.max_sent_packet).max().unwrap_or(0);
                println!(
                    "[тик {}] клиентов {} | тик сим {:.2} мс, худший тик с сетью {:.2} мс | \
                     на клиента: к нему {:.1} КБ/с, от него {:.1} КБ/с, пакет до {} Б | сервер отдаёт {:.0} КБ/с",
                    world.tick,
                    net.clients.len(),
                    world.stats.total_ms,
                    worst_ms,
                    down / 1024.0,
                    up / 1024.0,
                    max_packet,
                    net.traffic.sent_per_sec / 1024.0
                );
            }
            worst_ms = 0.0;
            if net.guard != guard_seen {
                guard_seen = net.guard;
                let g = net.guard;
                println!(
                    "  отбито всего: не расшифровались {}, с неверной печатью {}, слишком частых подключений {}, неверных паролей {}",
                    g.forged, g.bad_cookie, g.throttled, g.wrong_password
                );
            }
        }

        next += dt;
        let now = Instant::now();
        if next > now {
            std::thread::sleep(next - now);
        } else if now - next > Duration::from_millis(500) {
            next = now; // сильно отстали — не пытаемся догонять рывком
        }
    }
}

fn bench(config: SimConfig, args: &Args) {
    println!(
        "Карта {0}×{0} м, {1} солдат ({2} на команду), {3} тиков при {4} Гц, потоков: {5}",
        config.map_size,
        config.soldiers_per_team * 2,
        config.soldiers_per_team,
        args.ticks,
        config.tick_rate,
        rayon::current_num_threads(),
    );

    let started = Instant::now();
    let mut world = make_world(config, args);
    println!("Мир создан за {:.0} мс (карта и поля путей)", started.elapsed().as_secs_f32() * 1000.0);

    let mut history: Vec<TickStats> = Vec::with_capacity(args.ticks as usize);
    let run = Instant::now();
    for _ in 0..args.ticks {
        world.step();
        history.push(world.stats.clone());
    }
    let wall = run.elapsed().as_secs_f32();

    let mut totals: Vec<f32> = history.iter().map(|s| s.total_ms).collect();
    totals.sort_by(f32::total_cmp);
    let pct = |p: f32| totals[((totals.len() - 1) as f32 * p) as usize];
    let avg = |f: fn(&TickStats) -> f32| history.iter().map(f).sum::<f32>() / history.len() as f32;
    let budget = 1000.0 / world.config.tick_rate as f32;

    println!();
    println!("Время тика, мс (бюджет {budget:.0} мс):");
    println!(
        "  среднее {:.2} | медиана {:.2} | 99% {:.2} | максимум {:.2}",
        avg(|s| s.total_ms),
        pct(0.5),
        pct(0.99),
        totals[totals.len() - 1]
    );
    println!("По фазам, среднее:");
    type Phase = (&'static str, fn(&TickStats) -> f32);
    let phases: [Phase; 7] = [
        ("сетка", |s| s.grid_ms),
        ("пути", |s| s.nav_ms),
        ("ИИ", |s| s.think_ms),
        ("движение", |s| s.move_ms),
        ("стрельба", |s| s.fire_ms),
        ("снаряды", |s| s.projectiles_ms),
        ("точки", |s| s.capture_ms),
    ];
    for (name, f) in phases {
        println!("  {name:<10} {:.3}", avg(f));
    }
    println!();
    println!(
        "Симулировано {:.0} с боя за {:.1} с реального времени (×{:.0} быстрее реального)",
        args.ticks as f32 * world.dt(),
        wall,
        args.ticks as f32 * world.dt() / wall
    );
    println!(
        "Убийств: {} / {}, пуль в полёте в среднем: {:.0}",
        world.score[0],
        world.score[1],
        avg(|s| s.bullets as f32)
    );
    let owners: Vec<&str> = world
        .points
        .iter()
        .map(|p| match p.owner {
            Some(0) => "синие",
            Some(_) => "красные",
            None => "никто",
        })
        .collect();
    println!("Точки: {}", owners.join(", "));
}

/// Мир на карте из файла (`--map-file`) или на процедурной.
fn make_world(config: SimConfig, args: &Args) -> World {
    let Some(name) = &args.map_file else { return World::new(config, args.seed) };
    let maps = args.config.parent().unwrap_or(std::path::Path::new(".")).join("maps");
    match MapFile::load(&maps.join(name)) {
        Ok(file) => {
            println!("Карта «{}» из {}", file.name, maps.join(name).display());
            World::from_file(config, args.seed, file)
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(2);
        }
    }
}
