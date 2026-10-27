//! Штаб команды: бой сразу на нескольких точках, подкрепления — туда, где нужно,
//! и команда со штабом воюет лучше команды без него.

use wind_sim::{SimConfig, World};

fn world(seed: u64, command: [bool; 2]) -> World {
    let mut c = SimConfig { map_size: 512, soldiers_per_team: 120, capture_points: 5, ..Default::default() };
    // Раунд не кончается — меряем, сколько точек команда держит.
    c.round.hold_time = f32::INFINITY;
    c.round.time_limit = 0.0;
    let mut w = World::new(c, seed);
    w.command_enabled = command;
    w
}

/// К скольким разным точкам идут отряды команды.
fn fronts(w: &World, team: u8) -> usize {
    let mut points: Vec<u32> = w.squads.iter().filter(|q| q.team == team && q.leader.is_some()).map(|q| q.objective).collect();
    points.sort();
    points.dedup();
    points.len()
}

#[test]
fn command_works_on_several_points_at_once() {
    let mut with = world(3, [true; 2]);
    let mut without = world(3, [false; 2]);
    let (mut a, mut b) = (0.0, 0.0);
    for tick in 0..1200 {
        with.step();
        without.step();
        if tick % 100 == 99 {
            a += fronts(&with, 0) as f32 / 12.0;
            b += fronts(&without, 0) as f32 / 12.0;
        }
    }
    println!("отряды идут к точкам: со штабом в среднем к {a:.1} разным, без штаба — к {b:.1}");
    assert!(a > b + 0.5, "штаб ведёт бой на нескольких точках сразу: {a} против {b}");
    // Штаб видит каждую точку.
    assert_eq!(with.plans[0].len(), with.points.len());
    assert!(without.plans[0].is_empty());
}

#[test]
fn team_with_command_beats_team_without() {
    // Восемь боёв по 4 минуты; штаб — то у синих, то у красных, чтобы сторона не решала.
    let (mut held_with, mut held_without, mut kills_with, mut kills_without) = (0.0, 0.0, 0, 0);
    for seed in 1..=8u64 {
        let smart = (seed % 2) as usize;
        let mut command = [false; 2];
        command[smart] = true;
        let mut w = world(seed, command);
        for tick in 0..4800 {
            w.step();
            // Сколько точек держит каждая команда — в среднем за вторую половину боя.
            if tick >= 2400 && tick % 100 == 0 {
                for p in &w.points {
                    match p.owner {
                        Some(t) if t as usize == smart => held_with += 1.0 / 24.0,
                        Some(_) => held_without += 1.0 / 24.0,
                        None => {}
                    }
                }
            }
        }
        kills_with += w.score[smart];
        kills_without += w.score[1 - smart];
    }
    println!(
        "за 8 боёв: точек в среднем держит команда со штабом {:.1}, без штаба {:.1}; убийств {kills_with} против {kills_without}",
        held_with / 8.0,
        held_without / 8.0
    );
    // Бои по зерну одинаковы при каждом запуске; разница в полточки — заметная.
    assert!(held_with - held_without > 0.25 * 8.0, "со штабом команда держит больше точек");
}
