//! Нагрузочные боты: подключаются к серверу как обычные клиенты, бегают, стреляют
//! и считают трафик.
//!
//! cargo run --release -p wind_bots -- --count 24 --seconds 30

use std::time::{Duration, Instant};
use wind_net::{ClientStatus, DEFAULT_PORT, NetClient};
use wind_sim::rng::Rng;
use wind_sim::{PlayerInput, Vec2};

struct Args {
    server: String,
    count: u32,
    seconds: f32,
    loss: f32,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args { server: format!("127.0.0.1:{DEFAULT_PORT}"), count: 24, seconds: 30.0, loss: 0.0 };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().ok_or(format!("после {flag} нужно значение"));
        let number = |s: String| s.parse::<f32>().map_err(|_| format!("не число: {s}"));
        match flag.as_str() {
            "--server" => args.server = value()?,
            "--count" => args.count = number(value()?)? as u32,
            "--seconds" => args.seconds = number(value()?)?,
            "--loss" => args.loss = number(value()?)?,
            "--help" | "-h" => {
                println!("wind_bots [--server 127.0.0.1:7777] [--count 24] [--seconds 30] [--loss 0.1]");
                std::process::exit(0);
            }
            other => return Err(format!("неизвестный флаг {other}, см. --help")),
        }
    }
    Ok(args)
}

struct Bot {
    client: NetClient,
    rng: Rng,
    /// Боковое смещение, чтобы боты не шли гуськом.
    sway: f32,
}

impl Bot {
    fn think(&mut self) -> PlayerInput {
        let Some(world) = &self.client.world else { return PlayerInput::default() };
        let Some(me) = world.my_soldier() else { return PlayerInput::default() };
        let my_team = world.welcome.team;
        let pos = me.pos;

        // Ближайший враг в пределах дальности оружия.
        let enemy = world
            .soldiers
            .iter()
            .flatten()
            .filter(|s| s.net.team() != my_team)
            .map(|s| s.pos)
            .filter(|p| p.distance(pos) < world.welcome.weapon_range)
            .min_by(|a, b| a.distance(pos).total_cmp(&b.distance(pos)));

        // Идём к ближайшей не нашей точке.
        let goal = world
            .points
            .iter()
            .filter(|p| p.owner != Some(my_team))
            .map(|p| p.pos)
            .min_by(|a, b| a.distance(pos).total_cmp(&b.distance(pos)))
            .unwrap_or(world.points[0].pos);
        if self.rng.chance(0.02) {
            self.sway = self.rng.range(-0.8, 0.8);
        }
        let to = (goal - pos).normalize_or_zero();
        let move_dir = (to + to.perp() * self.sway).normalize_or_zero();

        match enemy {
            Some(e) => PlayerInput {
                move_dir: move_dir * 0.3,
                aim: e + Vec2::new(self.rng.range(-1.0, 1.0), self.rng.range(-1.0, 1.0)),
                fire: true,
                grenade: e.distance(pos) > 10.0 && self.rng.chance(0.01),
                pos: None,
            },
            None => PlayerInput { move_dir, aim: pos + to * 10.0, fire: false, grenade: false, pos: None },
        }
    }
}

fn main() {
    let args = parse_args().unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    });
    let mut bots: Vec<Bot> = (0..args.count)
        .map(|i| {
            let mut client = NetClient::connect(&args.server, &format!("Бот {}", i + 1), None).unwrap_or_else(|e| {
                eprintln!("не удалось подключиться к {}: {e}", args.server);
                std::process::exit(1);
            });
            client.simulated_loss = args.loss;
            Bot { client, rng: Rng::new(i as u64 + 1), sway: 0.0 }
        })
        .collect();
    println!(
        "{} ботов подключаются к {} на {} с, потери входящих: {:.0}%",
        args.count,
        args.server,
        args.seconds,
        args.loss * 100.0
    );

    let started = Instant::now();
    let mut last_report = Instant::now();
    let mut samples: Vec<f32> = Vec::new();
    let mut max_packet = 0;
    let tick = Duration::from_millis(50);
    while started.elapsed().as_secs_f32() < args.seconds {
        let frame = Instant::now();
        for bot in &mut bots {
            bot.client.poll();
            let input = bot.think();
            bot.client.send_input(input);
            for w in bot.client.world.iter_mut() {
                w.advance_effects(0.05);
            }
        }

        if last_report.elapsed() >= Duration::from_secs(2) {
            last_report = Instant::now();
            let connected: Vec<&Bot> = bots.iter().filter(|b| b.client.status == ClientStatus::Connected).collect();
            let n = connected.len().max(1) as f32;
            let down = connected.iter().map(|b| b.client.traffic.recv_per_sec).sum::<f32>() / n / 1024.0;
            let up = connected.iter().map(|b| b.client.traffic.sent_per_sec).sum::<f32>() / n / 1024.0;
            let rtt = connected.iter().map(|b| b.client.rtt_ms).sum::<f32>() / n;
            let errors: u32 = bots.iter().map(|b| b.client.decode_errors).sum();
            let seen = connected
                .iter()
                .filter_map(|b| b.client.world.as_ref())
                .map(|w| w.soldiers.iter().flatten().count())
                .sum::<usize>() as f32
                / n;
            let rejected: Vec<String> = bots
                .iter()
                .filter_map(|b| match &b.client.status {
                    ClientStatus::Rejected(why) => Some(why.clone()),
                    _ => None,
                })
                .collect();
            println!(
                "подключено {}/{} | на бота: к нему {down:.1} КБ/с, от него {up:.2} КБ/с, видит солдат {seen:.0} | \
                 пинг {rtt:.0} мс | не разобрано снимков {errors}{}",
                connected.len(),
                bots.len(),
                if rejected.is_empty() { String::new() } else { format!(" | отказ: {}", rejected[0]) }
            );
            if connected.len() == bots.len() && started.elapsed() > Duration::from_secs(3) {
                samples.push(down);
            }
        }
        max_packet = bots.iter().map(|b| b.client.traffic.max_recv_packet).max().unwrap_or(0).max(max_packet);
        if let Some(rest) = tick.checked_sub(frame.elapsed()) {
            std::thread::sleep(rest);
        }
    }

    if !samples.is_empty() {
        let avg = samples.iter().sum::<f32>() / samples.len() as f32;
        let worst = samples.iter().copied().fold(0.0, f32::max);
        println!("\nИтог: к боту в среднем {avg:.1} КБ/с, в худшие 2 с {worst:.1} КБ/с, самый большой пакет {max_packet} Б");
    }
    for bot in &mut bots {
        bot.client.disconnect();
    }
}
