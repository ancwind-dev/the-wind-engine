//! Игра. Одна против ИИ, по сети или замер FPS.
//!
//! cargo run --release -p wind_client                         — играть против ИИ
//! cargo run --release -p wind_client -- --connect 127.0.0.1:7777
//! cargo run --release -p wind_client -- --bench 15 --soldiers 1000
//! cargo run --release -p wind_client -- --export-models      — выгрузить модели в .vox

mod fog;
mod hud;
mod minimap;
mod source;

use eframe::egui::{self, Color32};
use eframe::egui_wgpu;
use source::{Local, Source};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use wind_net::{NetClient, Pilot};
use wind_render::{Camera, DrawInput, SceneBuilder, WorldPaint};
use wind_sim::mapfile::MapFile;
use wind_sim::{PlayerInput, SimConfig, Vec2, World};

/// MSAA. Должно совпадать в NativeOptions и в wind_render::install.
const MSAA: u32 = 4;
const SKY: Color32 = Color32::from_rgb(158, 173, 184);

struct Args {
    connect: Option<String>,
    name: String,
    soldiers: Option<u32>,
    map: Option<u32>,
    seed: u64,
    bench: Option<f32>,
    spectate: bool,
    /// За кем следит наблюдатель с самого начала.
    follow: u32,
    /// Карта из `data/maps/<имя>`.
    map_file: Option<String>,
    vsync: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args { connect: None, name: "Игрок".into(), soldiers: None, map: None, seed: 1, bench: None, spectate: false, follow: 0, map_file: None, vsync: false };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().ok_or(format!("после {flag} нужно значение"));
        let num = |s: String| s.parse::<f64>().map_err(|_| format!("не число: {s}"));
        match flag.as_str() {
            "--connect" => a.connect = Some(value()?),
            "--name" => a.name = value()?,
            "--soldiers" => a.soldiers = Some(num(value()?)? as u32),
            "--map" => a.map = Some(num(value()?)? as u32),
            "--seed" => a.seed = num(value()?)? as u64,
            "--bench" => a.bench = Some(num(value()?)? as f32),
            "--vsync" => a.vsync = true,
            "--spectate" => a.spectate = true,
            "--follow" => {
                a.spectate = true;
                a.follow = num(value()?)? as u32;
            }
            "--map-file" => a.map_file = Some(value()?),
            "--export-models" => {
                let dir = data_dir().join("models/soldier");
                match wind_render::models::export(&dir) {
                    Ok(files) => files.iter().for_each(|f| println!("сохранено: {f}")),
                    Err(e) => eprintln!("{e}"),
                }
                std::process::exit(0);
            }
            "--help" | "-h" => {
                println!(
                    "wind_client [--connect адрес] [--name имя] [--soldiers N на команду] [--map сторона] [--map-file имя] [--seed N]\n\
                     wind_client --bench секунд [--soldiers N]   — замер FPS, без игрока\n\
                     wind_client --spectate [--follow N]        — наблюдать за боем (Tab — следующий солдат)\n\
                     wind_client --export-models                — выгрузить модели солдата в data/models/soldier/*.vox\n\
                     --vsync — включить вертикальную синхронизацию (по умолчанию выключена, чтобы мерить FPS)"
                );
                std::process::exit(0);
            }
            other => return Err(format!("неизвестный флаг {other}, см. --help")),
        }
    }
    Ok(a)
}

/// Папка `data/`: от текущей папки, а если запустили не из корня — от корня проекта.
fn data_dir() -> PathBuf {
    let local = PathBuf::from("data");
    if local.join("sim.ron").exists() {
        return local;
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")
}

fn main() -> eframe::Result {
    let args = parse_args().unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    });
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_title("The Wind Engine").with_inner_size([1600.0, 900.0]),
        depth_buffer: 32,
        multisampling: MSAA as u16,
        wgpu_options: egui_wgpu::WgpuConfiguration {
            // Игре важнее плавность, чем минимальная задержка интерфейса.
            surface: egui_wgpu::SurfaceConfig {
                present_mode: if args.vsync {
                    eframe::wgpu::PresentMode::AutoVsync
                } else {
                    eframe::wgpu::PresentMode::AutoNoVsync
                },
                ..egui_wgpu::SurfaceConfig::HIGH_THROUGHPUT
            },
            ..Default::default()
        },
        ..Default::default()
    };
    eframe::run_native("wind_client", options, Box::new(|cc| Ok(Box::new(Game::new(cc, args)))))
}

/// Счётчик кадров: среднее за секунду и худший кадр.
struct Fps {
    frames: u32,
    window: Instant,
    worst: f32,
    pub fps: f32,
    pub worst_ms: f32,
}

impl Fps {
    fn tick(&mut self, dt: f32) {
        self.frames += 1;
        self.worst = self.worst.max(dt);
        let elapsed = self.window.elapsed().as_secs_f32();
        if elapsed >= 1.0 {
            self.fps = self.frames as f32 / elapsed;
            self.worst_ms = self.worst * 1000.0;
            self.frames = 0;
            self.worst = 0.0;
            self.window = Instant::now();
        }
    }
}

struct Bench {
    started: Instant,
    seconds: f32,
    samples: Vec<f32>,
    on_screen: Vec<usize>,
}

struct Game {
    source: Source,
    scene: SceneBuilder,
    camera: Camera,
    last_frame: Instant,
    started: Instant,
    fps: Fps,
    debug: bool,
    aim: Vec2,
    bench: Option<Bench>,
    /// За кем следит камера в режиме наблюдателя.
    follow: u32,
    /// Свой солдат на клиенте: двигается сразу, без ожидания сервера.
    pilot: Pilot,
    /// Отдача: 0 — спокойно, 1 — сильно. Растёт с каждым своим выстрелом и спадает.
    kick: f32,
    /// «Нагрев» своей очереди — как у сервера: разброс растёт с каждым выстрелом.
    heat: f32,
    /// Где появиться после смерти: 0 — база, 1 + i — точка i (клавиши 1–6).
    spawn: u8,
    fog: fog::Fog,
    minimap: minimap::Minimap,
    shots_seen: u32,
    notes: Vec<String>,
    render_state: Option<egui_wgpu::RenderState>,
}

impl Game {
    fn new(cc: &eframe::CreationContext<'_>, args: Args) -> Self {
        let mut notes = Vec::new();
        let mut characters = None;
        let render_state = cc.wgpu_render_state.clone();
        match &render_state {
            Some(rs) => {
                let installed = wind_render::install(rs, MSAA, Some(&data_dir()));
                notes.extend(installed.notes);
                characters = installed.characters;
                let info = rs.adapter.get_info();
                notes.push(format!("{} · {:?}", info.name, info.backend));
            }
            None => notes.push("wgpu недоступен — 3D не будет".into()),
        }

        let source = match &args.connect {
            Some(addr) => match NetClient::connect(addr, &args.name, None) {
                Ok(mut c) => {
                    c.maps_dir = data_dir().join("maps");
                    Source::Remote(c)
                }
                Err(e) => {
                    eprintln!("{addr}: {e}");
                    std::process::exit(1);
                }
            },
            None => {
                let mut config = SimConfig::load(&data_dir().join("sim.ron")).unwrap_or_else(|e| {
                    notes.push(format!("{e}; параметры по умолчанию"));
                    SimConfig::default()
                });
                if let Some(n) = args.soldiers {
                    config.soldiers_per_team = n;
                }
                if let Some(m) = args.map {
                    config.map_size = m;
                }
                let world = match &args.map_file {
                    Some(name) => match MapFile::load(&data_dir().join("maps").join(name)) {
                        Ok(file) => World::from_file(config, args.seed, file),
                        Err(e) => {
                            eprintln!("{e}");
                            std::process::exit(2);
                        }
                    },
                    None => World::new(config, args.seed),
                };
                Source::Local(Local::new(world, args.bench.is_none() && !args.spectate))
            }
        };

        let mut camera = Camera::default();
        let bench = args.bench.map(|seconds| {
            // Смотрим на всю карту сразу — в кадре почти все солдаты.
            if let Source::Local(l) = &source {
                let size = l.world.map.width as f32;
                camera.target = Vec2::splat(size * 0.5);
                camera.distance = size * 0.95;
            }
            Bench { started: Instant::now(), seconds, samples: Vec::new(), on_screen: Vec::new() }
        });

        Game {
            source,
            scene: SceneBuilder::with_characters(characters),
            camera,
            last_frame: Instant::now(),
            started: Instant::now(),
            fps: Fps { frames: 0, window: Instant::now(), worst: 0.0, fps: 0.0, worst_ms: 0.0 },
            debug: false,
            aim: Vec2::ZERO,
            bench,
            follow: args.follow,
            pilot: Pilot::default(),
            kick: 0.0,
            heat: 0.0,
            spawn: 0,
            fog: fog::Fog::default(),
            minimap: minimap::Minimap::default(),
            shots_seen: 0,
            notes,
            render_state,
        }
    }

    fn read_input(&mut self, ctx: &egui::Context, mouse_world: Option<Vec2>) -> PlayerInput {
        let (forward, right) = self.camera.screen_axes();
        if let Some(m) = mouse_world {
            self.aim = m;
        }
        // Где появиться: 1 — база, 2…6 — точки A…E.
        let keys = [egui::Key::Num1, egui::Key::Num2, egui::Key::Num3, egui::Key::Num4, egui::Key::Num5, egui::Key::Num6];
        if let Some(k) = ctx.input(|i| keys.iter().position(|&k| i.key_pressed(k))) {
            self.spawn = k as u8;
        }
        ctx.input(|i| {
            let mut d = Vec2::ZERO;
            if i.key_down(egui::Key::W) {
                d.y -= 1.0;
            }
            if i.key_down(egui::Key::S) {
                d.y += 1.0;
            }
            if i.key_down(egui::Key::A) {
                d.x -= 1.0;
            }
            if i.key_down(egui::Key::D) {
                d.x += 1.0;
            }
            PlayerInput {
                // Камера повёрнута, как в RWR, — WASD идут по экрану, а не по сторонам света.
                move_dir: (forward * -d.y + right * d.x).normalize_or_zero(),
                aim: self.aim,
                fire: i.pointer.primary_down(),
                grenade: i.key_down(egui::Key::G),
                spawn: self.spawn,
                // Заполняет пилот своего солдата.
                pos: None,
            }
        })
    }

    fn finish_bench(&mut self, ctx: &egui::Context) {
        let Some(b) = &self.bench else { return };
        if b.started.elapsed().as_secs_f32() < b.seconds {
            return;
        }
        // Первые 2 секунды — прогрев: строятся меши карты, грузятся шейдеры.
        let mut frame_ms: Vec<f32> = b.samples.iter().skip_while(|_| false).copied().collect();
        let warm = frame_ms.len() / 8;
        frame_ms.drain(..warm);
        let avg_ms = frame_ms.iter().sum::<f32>() / frame_ms.len().max(1) as f32;
        frame_ms.sort_by(f32::total_cmp);
        let p99 = frame_ms.get(frame_ms.len() * 99 / 100).copied().unwrap_or(0.0);
        let on_screen = b.on_screen.iter().sum::<usize>() as f32 / b.on_screen.len().max(1) as f32;
        let stats = self.render_state.as_ref().map(wind_render::stats).unwrap_or_default();
        println!("Замер {:.0} с, {} кадров:", b.seconds, frame_ms.len());
        println!("  FPS в среднем {:.0} (кадр {avg_ms:.2} мс), 1% худших кадров — {p99:.2} мс и дольше", 1000.0 / avg_ms.max(0.001));
        println!("  солдат на экране в среднем {on_screen:.0}");
        println!(
            "  последний кадр: экземпляров {}, чанков карты {} из {}, вызовов отрисовки {}",
            stats.instances, stats.chunks_drawn, stats.chunks, stats.draw_calls
        );
        for n in &self.notes {
            println!("  {n}");
        }
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        self.bench = None;
    }
}

impl eframe::App for Game {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let now = Instant::now();
        let dt = now.duration_since(self.last_frame).as_secs_f32();
        self.last_frame = now;
        self.fps.tick(dt);
        if ctx.input(|i| i.key_pressed(egui::Key::F3)) {
            self.debug = !self.debug;
        }

        egui::CentralPanel::no_frame().show(ui, |ui| {
            let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
            ui.painter().rect_filled(rect, 0, SKY);
            let aspect = rect.width() / rect.height().max(1.0);

            // Колесо — приблизить/отдалить.
            let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
            if response.hovered() && scroll != 0.0 {
                self.camera.distance = (self.camera.distance * (-scroll * 0.0015).exp()).clamp(12.0, 260.0);
            }
            let mouse_world = ctx.input(|i| i.pointer.hover_pos()).filter(|p| rect.contains(*p)).and_then(|p| {
                let ndc = Vec2::new((p.x - rect.left()) / rect.width() * 2.0 - 1.0, 1.0 - (p.y - rect.top()) / rect.height() * 2.0);
                match self.source.map() {
                    Some(map) => self.camera.pick(ndc, aspect, map),
                    None => self.camera.ground_point(ndc, aspect, 0.0),
                }
            });

            let mut input = if self.bench.is_some() { PlayerInput::default() } else { self.read_input(&ctx, mouse_world) };
            // В перерыве между раундами бой стоит — и свой солдат тоже.
            if !self.source.round_playing() {
                input.move_dir = Vec2::ZERO;
                input.fire = false;
                input.grenade = false;
            }
            // Свой солдат двигается сразу, а серверу уходит, где он (см. Pilot).
            if self.bench.is_none() {
                let (speed, radius) = self.source.soldier_spec();
                let server = self.source.own_server_pos();
                if let Some(map) = self.source.map() {
                    input.pos = self.pilot.update(map, server, input.move_dir, speed, radius, dt);
                }
                self.source.set_own_pos(input.pos);
            }
            self.source.update(dt, input);
            let mut view = self.source.view();
            if let (Some(p), Some(id)) = (self.pilot.pos(), self.source.own_id()) {
                view.me_pos = Some(p);
                let look = self.aim - p;
                for s in view.soldiers.iter_mut().filter(|s| s.id == id) {
                    s.pos = p;
                    if look != Vec2::ZERO {
                        s.facing = look.y.atan2(look.x);
                    }
                }
            }
            // Туман войны: враги — только те, кого видят свои.
            let own_team = view.round.own_team.filter(|_| view.has_me && self.bench.is_none());
            if let Some(map) = self.source.map() {
                let center = view.me_pos.unwrap_or(self.camera.target);
                self.fog.update(dt, map, self.source.vision_range(), own_team, center, &view.soldiers);
            }
            view.soldiers.retain(|s| self.fog.shows(s, own_team));
            if ctx.input(|i| i.key_pressed(egui::Key::M)) {
                self.minimap.big = !self.minimap.big;
            }
            // Отдача: каждый свой выстрел толкает прицел и камеру, потом всё успокаивается.
            let shots = self.source.own_shots();
            let weapon = self.source.own_weapon().unwrap_or_default();
            self.heat = (self.heat - weapon.sustained_fire_diminish_rate * dt).max(0.0);
            if shots > self.shots_seen {
                let n = (shots - self.shots_seen) as f32;
                self.kick = (self.kick + 0.3 * n).min(1.0);
                self.heat = (self.heat + weapon.sustained_fire_grow_step * n).min(wind_sim::arsenal::MAX_HEAT);
            }
            self.shots_seen = shots;
            self.kick *= (-dt * 7.0).exp();

            // Камера плавно идёт за своим солдатом, а у наблюдателя — за выбранным (Tab — следующий).
            if self.bench.is_none() && ctx.input(|i| i.key_pressed(egui::Key::Tab)) {
                self.follow += 1;
            }
            let focus = view.me_pos.or_else(|| {
                if self.bench.is_some() {
                    return None;
                }
                let next = view.soldiers.iter().filter(|s| s.id >= self.follow).min_by_key(|s| s.id);
                let s = next.or_else(|| view.soldiers.iter().min_by_key(|s| s.id))?;
                self.follow = s.id;
                Some(s.pos)
            });
            if let Some(me) = focus {
                // Своего солдата камера чуть выносит к прицелу — видно, куда стреляешь.
                let ahead = if view.me_pos.is_some() {
                    (self.aim - me).clamp_length_max(self.camera.distance * 0.6) * 0.3
                } else {
                    Vec2::ZERO
                };
                let k = 1.0 - (-dt * 8.0).exp();
                self.camera.target += (me + ahead - self.camera.target) * k;
                // Отдача чуть вздрагивает камерой.
                let t = self.started.elapsed().as_secs_f32();
                self.camera.target += Vec2::new((t * 91.0).sin(), (t * 73.0).cos()) * self.kick * 0.06;
                if let Some(map) = self.source.map() {
                    let h = map.height_at(self.camera.target);
                    self.camera.target_height += (h - self.camera.target_height) * k;
                }
            }

            if let Some(map) = self.source.map() {
                let ppp = ctx.pixels_per_point();
                let atmosphere = self.source.atmosphere();
                let time = self.started.elapsed().as_secs_f32();
                let input = DrawInput {
                    map,
                    map_key: self.source.map_key(),
                    camera: &self.camera,
                    aspect,
                    soldiers: &view.soldiers,
                    bullets: &view.bullets,
                    grenades: &view.grenades,
                    blasts: &view.blasts,
                    points: &view.points,
                    marker: view.me_pos,
                    rings: &[],
                    corpses: &view.corpses,
                    own_team,
                    atmosphere: &atmosphere,
                    viewport_px: [rect.left() * ppp, rect.top() * ppp, rect.width() * ppp, rect.height() * ppp],
                    time,
                    dt,
                };
                let frame = self.scene.build(&input);
                ui.painter().add(egui_wgpu::Callback::new_paint_callback(rect, WorldPaint(Arc::new(frame))));
            }

            if let Some(b) = &mut self.bench {
                b.samples.push(dt * 1000.0);
                let vp = self.camera.view_proj(aspect);
                let visible = view
                    .soldiers
                    .iter()
                    .filter(|s| {
                        let p = vp.project_point3(wind_render::camera::to_3d(s.pos, 1.0));
                        p.x.abs() <= 1.0 && p.y.abs() <= 1.0 && (0.0..=1.0).contains(&p.z)
                    })
                    .count();
                b.on_screen.push(visible);
            }

            let painter = ui.painter_at(rect);
            hud::draw(&painter, rect, &view, self.source.status(), !view.has_me);
            hud::round(&painter, rect, &view);
            if self.bench.is_none()
                && let Some(map) = self.source.map()
            {
                let facing = self.source.own_id().and_then(|id| view.soldiers.iter().find(|s| s.id == id)).map(|s| s.facing);
                self.minimap.draw(&ctx, &painter, rect, (map, self.source.map_key()), &view, facing);
            }
            if view.has_me && ctx.input(|i| i.key_down(egui::Key::Tab)) {
                hud::scoreboard(&painter, rect, &view);
            }
            if view.has_me && !view.me.alive && view.round.round.playing() {
                hud::spawn_choice(&painter, rect, &view, self.spawn);
            }
            if self.bench.is_none()
                && let Some(p) = ctx.input(|i| i.pointer.hover_pos())
            {
                let gap = self.spread_px(&view, rect, aspect);
                let time = self.started.elapsed().as_secs_f32();
                hud::crosshair(&painter, p, gap, view.me.alive && view.me.reloading, time);
            }
            let stats = self.render_state.as_ref().map(wind_render::stats).unwrap_or_default();
            hud::perf(&painter, rect, &self.fps_line(), self.debug.then(|| self.debug_lines(&stats, &view)));
        });

        self.finish_bench(&ctx);
        ctx.request_repaint();
    }
}

impl Game {
    /// Разброс оружия на расстоянии до прицела — в пикселях экрана: с ростом
    /// разброса в очереди и толчком отдачи.
    fn spread_px(&self, view: &source::View, rect: egui::Rect, aspect: f32) -> f32 {
        let (Some(weapon), Some(me), Some(map)) = (self.source.own_weapon(), view.me_pos, self.source.map()) else { return 4.0 };
        let to = self.aim - me;
        let half = to.length() * (weapon.spread(self.heat) * 0.5).tan();
        let side = to.normalize_or_zero().perp() * half;
        let vp = self.camera.view_proj(aspect);
        let screen = |p: Vec2| {
            let s = vp.project_point3(wind_render::camera::to_3d(p, map.height_at(p)));
            Vec2::new(s.x * rect.width() * 0.5, s.y * rect.height() * 0.5)
        };
        let px = screen(self.aim).distance(screen(self.aim + side));
        px.clamp(3.0, 160.0) + self.kick * 6.0
    }

    fn fps_line(&self) -> String {
        format!("{:.0} FPS · худший кадр {:.1} мс · F3 — отладка", self.fps.fps, self.fps.worst_ms)
    }

    fn debug_lines(&self, stats: &wind_render::RenderStats, view: &source::View) -> Vec<String> {
        let mut lines = vec![
            format!("Солдат в кадре: {}", view.soldiers.len()),
            format!("Экземпляров: {}, вызовов отрисовки: {}", stats.instances, stats.draw_calls),
            format!("Чанков карты: {} рисуется из {}", stats.chunks_drawn, stats.chunks),
            format!("Камера: {:.0} м", self.camera.distance),
            format!("Свой солдат: возвратов на место сервера {}", self.pilot.snaps),
        ];
        match &self.source {
            Source::Local(l) => {
                lines.push(format!("Симуляция: {:.2} мс на тик, {:.2} мс за кадр", l.world.stats.total_ms, l.sim_ms));
            }
            Source::Remote(c) => lines.push(format!(
                "Сеть: к нам {:.1} КБ/с, пинг {:.0} мс, не разобрано {}",
                c.traffic.recv_per_sec / 1024.0,
                c.rtt_ms,
                c.decode_errors
            )),
        }
        lines.extend(self.notes.iter().cloned());
        lines
    }
}
