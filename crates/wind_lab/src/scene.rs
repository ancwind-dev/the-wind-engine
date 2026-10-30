//! Сборка сцены для вьюпорта: из локальной симуляции или из мира, пришедшего по сети.

use crate::theme;
use crate::viewport::{Hud, Scene, ScenePoint, SceneSoldier, SelectedInfo};
use wind_net::NetClient;
use wind_sim::squad::Order;
use wind_sim::{Controller, ProjectileKind, Vec2, World};

/// Вспышка взрыва в локальной симуляции.
pub struct Blast {
    pub pos: Vec2,
    pub radius: f32,
    pub age: f32,
}

/// `ahead` — сколько секунд прошло после последнего тика (см. `Soldier::drawn_pos`).
pub fn local<'a>(world: &'a World, selected: Option<u32>, blasts: &[Blast], ahead: f32) -> Scene<'a> {
    let soldiers = world
        .soldiers
        .iter()
        .enumerate()
        .filter(|(_, s)| s.alive)
        .map(|(i, s)| SceneSoldier {
            id: i as u32,
            pos: s.drawn_pos(ahead),
            facing: s.facing,
            team: s.team,
            player: matches!(s.controller, Controller::Player(_)),
            look: None,
            prone: s.prone,
        })
        .collect();
    let mut bullets = Vec::new();
    let mut grenades = Vec::new();
    for p in &world.projectiles {
        match p.kind {
            ProjectileKind::Bullet => bullets.push((p.pos - p.vel * 0.04, p.pos)),
            ProjectileKind::Grenade { fuse, .. } => grenades.push((p.pos, fuse)),
            ProjectileKind::Shell { flight } => grenades.push((p.pos, flight)),
        }
    }
    let sel = selected.filter(|&i| world.soldiers[i as usize].alive);
    let (rings, lines) = sel.map(|i| squad_marks(world, i)).unwrap_or_default();
    Scene {
        title: "Вид сверху · локальная симуляция".into(),
        map: &world.map,
        bases: world.bases,
        points: world
            .points
            .iter()
            .map(|p| ScenePoint { pos: p.pos, owner: p.owner, progress: p.progress })
            .collect(),
        capture_radius: world.config.capture.radius,
        soldier_radius: world.config.soldier.radius,
        soldiers,
        bullets,
        grenades,
        blasts: blasts.iter().map(|b| (b.pos, b.radius, b.age)).collect(),
        selected: sel.map(|i| {
            let s = &world.soldiers[i as usize];
            SelectedInfo {
                pos: s.pos,
                ranges: Some((world.config.soldier.vision_range, world.weapon(i).range)),
                target: s.target.map(|t| &world.soldiers[t as usize]).filter(|t| t.alive).map(|t| t.pos),
            }
        }),
        flow: sel.and_then(|i| world.flow.get(world.soldiers[i as usize].objective as usize)),
        hud: None,
        kill_feed: Vec::new(),
        rings,
        lines,
        corpses: world
            .soldiers
            .iter()
            .enumerate()
            .filter(|(_, s)| !s.alive && s.respawn_timer > 0.0)
            .map(|(i, s)| wind_render::CorpseDraw {
                id: i as u32,
                pos: s.pos,
                facing: s.facing,
                team: s.team,
                age: world.config.soldier.respawn_time - s.respawn_timer,
                push: s.death_push,
                blast: s.death_blast,
                model: None,
            })
            .collect(),
        atmosphere: world.atmosphere.clone(),
    }
}

/// Кольцо на земле: центр, радиус, цвет sRGB 0..1.
type Ring = (Vec2, f32, [f32; 3]);

/// Отряд и укрытие выбранного солдата: командир — оранжевое кольцо, места в строю —
/// маленькие белые кольца, от командира к бойцам — линии (в 2D). Укрытие: откуда
/// стрелять — зелёное кольцо, где прятаться — серое, между ними и к угрозе — линии.
fn squad_marks(world: &World, selected: u32) -> (Vec<Ring>, Vec<(Vec2, Vec2)>) {
    let (mut rings, mut lines) = (Vec::new(), Vec::new());
    let me = &world.soldiers[selected as usize];
    if let Some(c) = me.cover {
        rings.push((c.shoot, 0.55, [0.35, 0.85, 0.35]));
        rings.push((c.hide, 0.55, [0.55, 0.55, 0.55]));
        lines.push((c.shoot, c.hide));
        lines.push((c.shoot, c.threat));
    }
    let Some(squad) = world.squads.get(me.squad as usize) else { return (rings, lines) };
    let Some(leader) = squad.leader.map(|l| world.soldiers[l as usize].pos) else { return (rings, lines) };
    rings.push((leader, 1.3, [1.0, 0.63, 0.16]));
    for &m in &squad.members {
        let s = &world.soldiers[m as usize];
        if let (true, Some(Order::Follow { slot, .. })) = (s.alive, world.orders.get(m as usize)) {
            rings.push((*slot, 0.45, [0.9, 0.9, 0.9]));
            lines.push((leader, s.pos));
        }
    }
    (rings, lines)
}

/// `None`, пока сервер не прислал приветствие.
pub fn remote(client: &NetClient) -> Option<Scene<'_>> {
    let w = client.world.as_ref()?;
    let now = client.now();
    let soldiers: Vec<SceneSoldier> = w
        .soldiers
        .iter()
        .enumerate()
        .filter_map(|(i, s)| {
            let s = s.as_ref()?;
            Some(SceneSoldier {
                id: i as u32,
                pos: w.render_pos(s, now),
                facing: s.net.facing(),
                team: s.net.team(),
                player: s.net.is_player(),
                look: None,
                prone: s.net.is_prone(),
            })
        })
        .collect();
    let me = w.my_soldier().map(|s| w.render_pos(s, now));
    let name = |id: u32| if id == w.welcome.soldier { "Ты".to_string() } else { format!("#{id}") };
    Some(Scene {
        title: format!("Игра по сети · {}", client.server_addr()),
        map: &w.map,
        bases: w.bases,
        points: w
            .points
            .iter()
            .map(|p| ScenePoint { pos: p.pos, owner: p.owner, progress: p.progress })
            .collect(),
        capture_radius: w.welcome.capture_radius,
        soldier_radius: w.welcome.soldier_radius,
        soldiers,
        bullets: w.tracers.iter().map(|t| (t.pos - t.vel * 0.04, t.pos)).collect(),
        grenades: w.grenades.iter().map(|g| (g.pos, g.fuse)).collect(),
        blasts: w.blasts.clone(),
        selected: me.map(|pos| SelectedInfo { pos, ranges: Some((w.welcome.interest_radius, w.my_weapon().range)), target: None }),
        flow: None,
        hud: Some(Hud {
            alive: w.me.alive,
            health: w.me.health,
            ammo: w.me.ammo,
            magazine: w.my_weapon().magazine_size,
            reloading: w.me.reloading,
            grenades: w.me.grenades,
            respawn_in: w.me.respawn_in,
            kills: w.me.kills,
        }),
        kill_feed: w
            .kill_feed
            .iter()
            .map(|k| (format!("{} → {}", name(k.killer), name(k.victim)), theme::TEAM[k.killer_team as usize]))
            .collect(),
        rings: Vec::new(),
        lines: Vec::new(),
        corpses: w.corpses.iter().map(|c| wind_render::CorpseDraw { id: c.id, pos: c.pos, facing: c.facing, team: c.team, age: c.age, push: c.push, blast: c.blast, model: None }).collect(),
        atmosphere: w.atmosphere.clone(),
    })
}
