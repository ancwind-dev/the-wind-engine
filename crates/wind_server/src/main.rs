//! Сервер без окна. Пока что — замер производительности симуляции.
//!
//! cargo run --release -p wind_server -- --soldiers 1000 --ticks 2000

use std::path::PathBuf;
use std::time::Instant;
use wind_sim::{SimConfig, TickStats, World};

struct Args {
    config: PathBuf,
    soldiers: Option<u32>,
    map: Option<u32>,
    ticks: u64,
    seed: u64,
}

fn parse_args() -> Result<Args, String> {
    let mut args = Args {
        config: PathBuf::from("data/sim.ron"),
        soldiers: None,
        map: None,
        ticks: 2000,
        seed: 1,
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
            "--help" | "-h" => {
                println!(
                    "wind_server [--config data/sim.ron] [--soldiers N на команду] \
                     [--map сторона, м] [--ticks N] [--seed N]"
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
    let mut world = World::new(config, args.seed);
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
    for (name, f) in [
        ("сетка", (|s: &TickStats| s.grid_ms) as fn(&TickStats) -> f32),
        ("пути", |s| s.nav_ms),
        ("ИИ", |s| s.think_ms),
        ("движение", |s| s.move_ms),
        ("стрельба", |s| s.fire_ms),
        ("снаряды", |s| s.projectiles_ms),
        ("точки", |s| s.capture_ms),
    ] {
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
