//! Игра: главное меню, бой против ИИ, по сети, наблюдение и замер FPS.
//!
//! cargo run --release -p wind_client                         — главное меню
//! cargo run --release -p wind_client -- --play               — сразу в бой против ИИ
//! cargo run --release -p wind_client -- --connect 127.0.0.1:7777
//! cargo run --release -p wind_client -- --bench 15 --soldiers 1000
//! cargo run --release -p wind_client -- --export-models      — выгрузить модели в .vox
//! cargo run --release -p wind_client -- --export-sounds      — звуки в WAV, в notes/Звуки

mod fog;
mod hud;
mod menu;
mod minimap;
mod settings;
mod sound;
mod source;

use eframe::egui::{self, Color32};
use eframe::egui_wgpu;
use menu::{Action, Menu};
use settings::Settings;
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

/// Флаги запуска. Имя, громкость, солдаты и карта меняют настройки только на этот
/// запуск (в файл они попадут, если потом поменять настройки в меню).
struct Args {
    connect: Option<String>,
    name: Option<String>,
    soldiers: Option<u32>,
    map: Option<u32>,
    seed: Option<u64>,
    bench: Option<f32>,
    /// Сразу в бой против ИИ, без меню.
    play: bool,
    spectate: bool,
    /// За кем следит наблюдатель с самого начала.
    follow: u32,
    /// Карта из `data/maps/<имя>`.
    map_file: Option<String>,
    vsync: bool,
    volume: Option<f32>,
    mute: bool,
}

fn parse_args() -> Result<Args, String> {
    let mut a = Args {
        connect: None,
        name: None,
        soldiers: None,
        map: None,
        seed: None,
        bench: None,
        play: false,
        spectate: false,
        follow: 0,
        map_file: None,
        vsync: false,
        volume: None,
        mute: false,
    };
    let mut it = std::env::args().skip(1);
    while let Some(flag) = it.next() {
        let mut value = || it.next().ok_or(format!("после {flag} нужно значение"));
        let num = |s: String| s.parse::<f64>().map_err(|_| format!("не число: {s}"));
        match flag.as_str() {
            "--connect" => a.connect = Some(value()?),
            "--name" => a.name = Some(value()?),
            "--soldiers" => a.soldiers = Some(num(value()?)? as u32),
            "--map" => a.map = Some(num(value()?)? as u32),
            "--seed" => a.seed = Some(num(value()?)? as u64),
            "--bench" => a.bench = Some(num(value()?)? as f32),
            "--play" => a.play = true,
            "--vsync" => a.vsync = true,
            "--volume" => a.volume = Some((num(value()?)? as f32).clamp(0.0, 1.0)),
            "--mute" => a.mute = true,
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
            "--export-sounds" => {
                export_sounds();
                std::process::exit(0);
            }
            "--help" | "-h" => {
                println!(
                    "wind_client                                — главное меню (настройки — Esc → Настройки)\n\
                     wind_client --play                         — сразу в бой против ИИ\n\
                     wind_client --connect адрес [--name имя]   — сразу на сервер\n\
                     wind_client --spectate [--follow N]        — наблюдать за боем (Tab — следующий солдат)\n\
                     wind_client --bench секунд [--soldiers N]   — замер FPS, без игрока\n\
                     wind_client --export-models                — выгрузить модели солдата в data/models/soldier/*.vox\n\
                     wind_client --export-sounds                — сохранить все звуки в notes/Звуки/*.wav\n\
                     --soldiers N на команду, --map сторона, --map-file имя, --seed N — бой против ИИ на этот запуск\n\
                     --volume 0..1 — громкость, --mute — без звука\n\
                     --vsync — включить вертикальную синхронизацию (по умолчанию выключена, чтобы мерить FPS)\n\
                     Настройки: {}",
                    Settings::path().display()
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

/// Все звуки и их варианты — в WAV, чтобы послушать без игры.
fn export_sounds() {
    let dir = data_dir().join("../notes/Звуки");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("{}: {e}", dir.display());
        return;
    }
    let rate = 48_000;
    for sfx in wind_audio::Sfx::ALL {
        for v in 0..sfx.variants() {
            let name = if sfx.variants() > 1 { format!("{}-{}.wav", sfx.name(), v + 1) } else { format!("{}.wav", sfx.name()) };
            let path = dir.join(name);
            let samples = wind_audio::synth::render(sfx, v, rate);
            let saved = std::fs::File::create(&path).and_then(|f| wind_audio::wav::write(&mut std::io::BufWriter::new(f), &samples, rate));
            match saved {
                Ok(()) => println!("сохранено: {}", path.display()),
                Err(e) => eprintln!("{}: {e}", path.display()),
            }
        }
    }
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

/// Один бой: откуда мир, камера и всё, что про своего солдата. Новый бой —
/// новая `Session`, ничего не тянется из прошлого.
struct Session {
    source: Source,
    /// Фон главного меню: бой ИИ без интерфейса и звука.
    backdrop: bool,
    camera: Camera,
    aim: Vec2,
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
    /// Сколько ещё секунд просить перезарядку после нажатия R: короткое нажатие
    /// не должно потеряться между пакетами ввода и тиками сервера.
    reload_hold: f32,
    fog: fog::Fog,
    minimap: minimap::Minimap,
    shots_seen: u32,
    sound: sound::Sound,
}

impl Session {
    fn new(source: Source) -> Session {
        Session {
            source,
            backdrop: false,
            camera: Camera::default(),
            aim: Vec2::ZERO,
            follow: 0,
            pilot: Pilot::default(),
            kick: 0.0,
            heat: 0.0,
            spawn: 0,
            reload_hold: 0.0,
            fog: fog::Fog::default(),
            minimap: minimap::Minimap::default(),
            shots_seen: 0,
            sound: sound::Sound::default(),
        }
    }

    /// Бой против ИИ по настройкам: `play` — со своим солдатом, иначе наблюдаем.
    fn local(settings: &Settings, seed: u64, play: bool, notes: &mut Vec<String>) -> Result<Session, String> {
        let mut config = SimConfig::load(&data_dir().join("sim.ron")).unwrap_or_else(|e| {
            notes.push(format!("{e}; параметры по умолчанию"));
            SimConfig::default()
        });
        config.soldiers_per_team = settings.soldiers_per_team;
        config.map_size = settings.map_size;
        let world = if settings.map_file.is_empty() {
            World::new(config, seed)
        } else {
            let file = MapFile::load(&data_dir().join("maps").join(&settings.map_file)).map_err(|e| format!("карта «{}»: {e}", settings.map_file))?;
            World::from_file(config, seed, file)
        };
        Ok(Session::new(Source::Local(Box::new(Local::new(world, play)))))
    }

    /// Фон меню: небольшой бой ИИ, камера ходит за солдатами.
    fn backdrop(seed: u64) -> Session {
        let settings = Settings { soldiers_per_team: 60, map_size: 192, map_file: String::new(), ..Default::default() };
        let mut s = Session::local(&settings, seed, false, &mut Vec::new()).expect("случайная карта строится всегда");
        s.backdrop = true;
        s.camera.distance = 55.0;
        s
    }

    fn remote(settings: &Settings) -> Result<Session, String> {
        let server = settings.server.trim();
        let mut c = NetClient::connect(server, &settings.name, None, &settings.password).map_err(|e| format!("{server}: {e}"))?;
        c.maps_dir = data_dir().join("maps");
        Ok(Session::new(Source::Remote(Box::new(c))))
    }

    fn read_input(&mut self, ctx: &egui::Context, mouse_world: Option<Vec2>, dt: f32) -> PlayerInput {
        let (forward, right) = self.camera.screen_axes();
        if let Some(m) = mouse_world {
            self.aim = m;
        }
        self.reload_hold = if ctx.input(|i| i.key_pressed(egui::Key::R)) { 0.25 } else { (self.reload_hold - dt).max(0.0) };
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
                reload: self.reload_hold > 0.0 || i.key_down(egui::Key::R),
                spawn: self.spawn,
                // Заполняет пилот своего солдата.
                pos: None,
            }
        })
    }

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
}

struct Game {
    session: Session,
    menu: Menu,
    settings: Settings,
    /// Какая громкость сейчас у звука — чтобы слать новую только при изменении.
    volume_applied: f32,
    /// `--seed`: тогда каждый бой с ним, иначе каждый раз новый.
    seed: Option<u64>,
    /// После меню кнопка мыши ещё зажата — не стрелять, пока не отпустят.
    fire_lock: bool,
    /// Короткая надпись по центру сверху (что переключили) и когда появилась.
    toast: Option<(String, Instant)>,
    scene: SceneBuilder,
    last_frame: Instant,
    started: Instant,
    fps: Fps,
    debug: bool,
    bench: Option<Bench>,
    audio: wind_audio::Audio,
    notes: Vec<String>,
    render_state: Option<egui_wgpu::RenderState>,
}

/// Новый бой — новое зерно карты (если не задано флагом).
fn fresh_seed() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64)
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

        // Настройки: файл, а без него — числа боя из sim.ron. Флаги — поверх.
        let sim = SimConfig::load(&data_dir().join("sim.ron")).unwrap_or_default();
        let defaults = Settings { soldiers_per_team: sim.soldiers_per_team, map_size: sim.map_size, ..Default::default() };
        let (mut settings, error) = Settings::load(&Settings::path(), defaults);
        notes.extend(error);
        if let Some(n) = &args.name {
            settings.name = n.clone();
        }
        if let Some(n) = args.soldiers {
            settings.soldiers_per_team = n;
        }
        if let Some(m) = args.map {
            settings.map_size = m;
        }
        if let Some(f) = &args.map_file {
            settings.map_file = f.clone();
        }
        if let Some(v) = args.volume {
            settings.volume = v;
        }

        // Флаги режима — сразу в бой, иначе главное меню над боем ИИ.
        let seed = args.seed.or(args.bench.map(|_| 1));
        let mut menu_open = false;
        let mut session = if let Some(addr) = &args.connect {
            settings.server = addr.clone();
            Session::remote(&settings).unwrap_or_else(|e| {
                eprintln!("{e}");
                std::process::exit(1);
            })
        } else if args.bench.is_some() || args.spectate || args.play {
            Session::local(&settings, seed.unwrap_or(1), args.play && args.bench.is_none() && !args.spectate, &mut notes).unwrap_or_else(|e| {
                eprintln!("{e}");
                std::process::exit(2);
            })
        } else {
            menu_open = true;
            Session::backdrop(fresh_seed())
        };
        session.follow = args.follow;

        // В замере звук ни к чему.
        let audio = if args.mute || args.bench.is_some() { wind_audio::Audio::silent() } else { wind_audio::Audio::open(settings.volume) };
        if !audio.is_on() {
            notes.push("звук выключен".into());
        }

        let bench = args.bench.map(|seconds| {
            // Смотрим на всю карту сразу — в кадре почти все солдаты.
            if let Source::Local(l) = &session.source {
                let size = l.world.map.width as f32;
                session.camera.target = Vec2::splat(size * 0.5);
                session.camera.distance = size * 0.95;
            }
            Bench { started: Instant::now(), seconds, samples: Vec::new(), on_screen: Vec::new() }
        });

        let maps = menu::list_maps(&data_dir().join("maps"));
        Game {
            session,
            menu: Menu::new(menu_open, maps),
            volume_applied: settings.volume,
            settings,
            seed,
            fire_lock: false,
            toast: None,
            scene: SceneBuilder::with_characters(characters),
            last_frame: Instant::now(),
            started: Instant::now(),
            fps: Fps { frames: 0, window: Instant::now(), worst: 0.0, fps: 0.0, worst_ms: 0.0 },
            debug: false,
            bench,
            audio,
            notes,
            render_state,
        }
    }

    fn save_settings(&mut self) {
        if let Err(e) = self.settings.save(&Settings::path()) {
            self.menu.error = Some(format!("Настройки не сохранились: {e}"));
        }
    }

    /// Выбор в меню.
    fn act(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::None => {}
            Action::PlayLocal | Action::Spectate => {
                let seed = self.seed.unwrap_or_else(fresh_seed);
                match Session::local(&self.settings, seed, action == Action::PlayLocal, &mut self.notes) {
                    Ok(s) => {
                        self.session = s;
                        self.menu.open = false;
                        self.fire_lock = true;
                    }
                    Err(e) => self.menu.error = Some(e),
                }
            }
            Action::Connect => {
                // Адрес и имя запомним, даже если не подключились: поправить опечатку проще.
                self.save_settings();
                match Session::remote(&self.settings) {
                    Ok(s) => {
                        self.session = s;
                        self.menu.open = false;
                        self.fire_lock = true;
                    }
                    Err(e) => self.menu.error = Some(e),
                }
            }
            Action::Resume => {
                self.menu.open = false;
                self.fire_lock = true;
            }
            Action::ToMainMenu => {
                // Старая сессия уходит — клиент сети при этом прощается с сервером.
                self.session = Session::backdrop(fresh_seed());
                self.menu.page = menu::Page::Main;
            }
            Action::SettingsClosed => self.save_settings(),
            Action::Quit => ctx.send_viewport_cmd(egui::ViewportCommand::Close),
        }
    }

    fn finish_bench(&mut self, ctx: &egui::Context) {
        let Some(b) = &self.bench else { return };
        if b.started.elapsed().as_secs_f32() < b.seconds {
            return;
        }
        // Первые 2 секунды — прогрев: строятся меши карты, грузятся шейдеры.
        let mut frame_ms: Vec<f32> = b.samples.clone();
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
        // F4 — солдаты «точно как RWR» или «как было»: сравнить и, если не понравится,
        // вернуть прежний вид.
        if ctx.input(|i| i.key_pressed(egui::Key::F4)) {
            self.scene.rwr_exact = !self.scene.rwr_exact;
            let text = if self.scene.rwr_exact { "Солдаты: точно как RWR (F4)" } else { "Солдаты: как было (F4)" };
            self.toast = Some((text.into(), Instant::now()));
        }
        let in_match = !self.session.backdrop;
        if self.bench.is_none() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            let action = self.menu.escape(in_match);
            self.act(action, &ctx);
        }

        egui::CentralPanel::no_frame().show(ui, |ui| self.world_frame(ui, &ctx, dt));

        let in_match = !self.session.backdrop;
        let action = self.menu.show(&ctx, &mut self.settings, in_match);
        self.act(action, &ctx);
        if self.settings.volume != self.volume_applied {
            self.audio.set_volume(self.settings.volume);
            self.volume_applied = self.settings.volume;
        }

        self.finish_bench(&ctx);
        ctx.request_repaint();
    }
}

impl Game {
    /// Бой: ввод, мир, камера, отрисовка, звук и HUD. В паузе своя игра стоит
    /// (по сети сервер не ждёт — стоит только свой солдат).
    fn world_frame(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, dt: f32) {
        let s = &mut self.session;
        let paused = !s.backdrop && self.menu.open;
        let controls = self.bench.is_none() && !s.backdrop && !paused;
        let (rect, response) = ui.allocate_exact_size(ui.available_size(), egui::Sense::click_and_drag());
        ui.painter().rect_filled(rect, 0, SKY);
        let aspect = rect.width() / rect.height().max(1.0);

        // Со своим солдатом камера, как в RWR, на одном расстоянии (36 м); колесо
        // приближает и отдаляет только у наблюдателя и пока ждёшь появления.
        if s.pilot.pos().is_some() && s.source.own_id().is_some() {
            let k = 1.0 - (-dt * 8.0).exp();
            s.camera.distance += (Camera::default().distance - s.camera.distance) * k;
        } else {
            let scroll = ctx.input(|i| i.smooth_scroll_delta.y);
            if controls && response.hovered() && scroll != 0.0 {
                s.camera.distance = (s.camera.distance * (-scroll * 0.0015).exp()).clamp(12.0, 260.0);
            }
        }
        let mouse_world = ctx.input(|i| i.pointer.hover_pos()).filter(|p| rect.contains(*p)).and_then(|p| {
            let ndc = Vec2::new((p.x - rect.left()) / rect.width() * 2.0 - 1.0, 1.0 - (p.y - rect.top()) / rect.height() * 2.0);
            match s.source.map() {
                Some(map) => s.camera.pick(ndc, aspect, map),
                None => s.camera.ground_point(ndc, aspect, 0.0),
            }
        });

        let mut input = if controls { s.read_input(ctx, mouse_world, dt) } else { PlayerInput { aim: s.aim, spawn: s.spawn, ..Default::default() } };
        if self.fire_lock {
            if input.fire {
                input.fire = false;
            } else {
                self.fire_lock = false;
            }
        }
        // В перерыве между раундами бой стоит — и свой солдат тоже.
        if !s.source.round_playing() {
            input.move_dir = Vec2::ZERO;
            input.fire = false;
            input.grenade = false;
            input.reload = false;
        }
        // Свой солдат двигается сразу, а серверу уходит, где он (см. Pilot).
        if self.bench.is_none() {
            let (speed, radius) = s.source.soldier_spec();
            let server = s.source.own_server_pos();
            if let Some(map) = s.source.map() {
                input.pos = s.pilot.update(map, server, input.move_dir, speed, radius, dt);
            }
            s.source.set_own_pos(input.pos);
        }
        // Своя игра в паузе стоит: время мира не идёт.
        let sim_dt = if paused && matches!(s.source, Source::Local(_)) { 0.0 } else { dt };
        s.source.update(sim_dt, input);
        let mut view = s.source.view();
        if let (Some(p), Some(id)) = (s.pilot.pos(), s.source.own_id()) {
            view.me_pos = Some(p);
            let look = s.aim - p;
            for soldier in view.soldiers.iter_mut().filter(|x| x.id == id) {
                soldier.pos = p;
                if look != Vec2::ZERO {
                    soldier.facing = look.y.atan2(look.x);
                }
            }
        }
        // Туман войны: враги — только те, кого видят свои.
        let own_team = view.round.own_team.filter(|_| view.has_me && self.bench.is_none());
        if let Some(map) = s.source.map() {
            let center = view.me_pos.unwrap_or(s.camera.target);
            s.fog.update(dt, map, s.source.vision_range(), own_team, center, &view.soldiers);
        }
        view.soldiers.retain(|x| s.fog.shows(x, own_team));
        if controls && ctx.input(|i| i.key_pressed(egui::Key::M)) {
            s.minimap.big = !s.minimap.big;
        }
        // Отдача: каждый свой выстрел раздвигает прицел, потом он успокаивается. Камеру
        // отдача не трогает — тряска мешала целиться.
        let shots = s.source.own_shots();
        let weapon = s.source.own_weapon().unwrap_or_default();
        s.heat = (s.heat - weapon.sustained_fire_diminish_rate * dt).max(0.0);
        if shots > s.shots_seen {
            let n = (shots - s.shots_seen) as f32;
            s.kick = (s.kick + 0.3 * n).min(1.0);
            s.heat = (s.heat + weapon.sustained_fire_grow_step * n).min(wind_sim::arsenal::MAX_HEAT);
        }
        s.shots_seen = shots;
        s.kick *= (-dt * 7.0).exp();

        // Камера плавно идёт за своим солдатом, а у наблюдателя — за выбранным (Tab — следующий).
        if controls && ctx.input(|i| i.key_pressed(egui::Key::Tab)) {
            s.follow += 1;
        }
        let bench = self.bench.is_some();
        let focus = view.me_pos.or_else(|| {
            if bench {
                return None;
            }
            let next = view.soldiers.iter().filter(|x| x.id >= s.follow).min_by_key(|x| x.id);
            let x = next.or_else(|| view.soldiers.iter().min_by_key(|x| x.id))?;
            s.follow = x.id;
            Some(x.pos)
        });
        if let Some(me) = focus {
            // Своего солдата камера чуть выносит к прицелу — видно, куда стреляешь.
            let ahead = if view.me_pos.is_some() {
                (s.aim - me).clamp_length_max(s.camera.distance * 0.6) * 0.3
            } else {
                Vec2::ZERO
            };
            // Фон меню — медленнее, чтобы не дёргало.
            let k = 1.0 - (-dt * if s.backdrop { 1.5 } else { 8.0 }).exp();
            s.camera.target += (me + ahead - s.camera.target) * k;
            if let Some(map) = s.source.map() {
                let h = map.height_at(s.camera.target);
                s.camera.target_height += (h - s.camera.target_height) * k;
            }
        }

        if let Some(map) = s.source.map() {
            let ppp = ctx.pixels_per_point();
            let atmosphere = s.source.atmosphere();
            let time = self.started.elapsed().as_secs_f32();
            let input = DrawInput {
                map,
                map_key: s.source.map_key(),
                camera: &s.camera,
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

        // Звук: слышно и тех, кого не видно за туманом. Фон меню — молча.
        let heard = s.source.take_heard();
        if self.audio.is_on() && !s.backdrop {
            let ears = sound::Ears {
                pos: view.me_pos.unwrap_or(s.camera.target),
                right: s.camera.screen_axes().1,
                own_id: s.source.own_id().filter(|_| view.has_me),
                own_pos: view.me_pos,
            };
            let source = &s.source;
            let gun_of = |team: u8, shooter: u32| source.weapon_of(team, shooter).map_or(wind_audio::Gun::Rifle, sound::gun);
            let plays = s.sound.frame(&ears, &view, &heard, &self.scene.crumbled, gun_of);
            sound::play_all(&self.audio, plays);
        }

        if let Some(b) = &mut self.bench {
            b.samples.push(dt * 1000.0);
            let vp = s.camera.view_proj(aspect);
            let visible = view
                .soldiers
                .iter()
                .filter(|x| {
                    let p = vp.project_point3(wind_render::camera::to_3d(x.pos, 1.0));
                    p.x.abs() <= 1.0 && p.y.abs() <= 1.0 && (0.0..=1.0).contains(&p.z)
                })
                .count();
            b.on_screen.push(visible);
        }

        let painter = ui.painter_at(rect);
        if !s.backdrop {
            hud::draw(&painter, rect, &view, s.source.status(), !view.has_me);
            hud::round(&painter, rect, &view);
            if !bench && let Some(map) = s.source.map() {
                let facing = s.source.own_id().and_then(|id| view.soldiers.iter().find(|x| x.id == id)).map(|x| x.facing);
                s.minimap.draw(ctx, &painter, rect, (map, s.source.map_key()), &view, facing);
            }
            if controls && view.has_me && ctx.input(|i| i.key_down(egui::Key::Tab)) {
                hud::scoreboard(&painter, rect, &view);
            }
            if view.has_me && !view.me.alive && view.round.round.playing() {
                hud::spawn_choice(&painter, rect, &view, s.spawn);
            }
            if controls && let Some(p) = ctx.input(|i| i.pointer.hover_pos()) {
                let gap = s.spread_px(&view, rect, aspect);
                let time = self.started.elapsed().as_secs_f32();
                hud::crosshair(&painter, p, gap, view.me.alive && view.me.reloading, time);
            }
        }
        let stats = self.render_state.as_ref().map(wind_render::stats).unwrap_or_default();
        let lines = self.debug.then(|| self.debug_lines(&stats, &view));
        hud::perf(&painter, rect, &self.fps_line(), lines);
        if let Some((text, at)) = &self.toast {
            if at.elapsed().as_secs_f32() < 2.0 {
                hud::toast(&painter, rect, text);
            } else {
                self.toast = None;
            }
        }
    }

    fn fps_line(&self) -> String {
        format!("{:.0} FPS · худший кадр {:.1} мс · F3 — отладка", self.fps.fps, self.fps.worst_ms)
    }

    fn debug_lines(&self, stats: &wind_render::RenderStats, view: &source::View) -> Vec<String> {
        let s = &self.session;
        let mut lines = vec![
            format!("Солдат в кадре: {}", view.soldiers.len()),
            format!("Экземпляров: {}, вызовов отрисовки: {}", stats.instances, stats.draw_calls),
            format!("Чанков карты: {} рисуется из {}", stats.chunks_drawn, stats.chunks),
            format!("Камера: {:.0} м", s.camera.distance),
            format!("Солдаты: {} (F4)", if self.scene.rwr_exact { "точно как RWR" } else { "как было" }),
            format!("Свой солдат: возвратов на место сервера {}", s.pilot.snaps),
        ];
        if self.audio.is_on() {
            let (voices, peak) = self.audio.meter();
            lines.push(format!("Звук: голосов {voices} из {}, пик {peak:.2}", wind_audio::mixer::MAX_VOICES));
        }
        match &s.source {
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
        lines.push(format!("Настройки: {}", Settings::path().display()));
        lines.extend(self.notes.iter().cloned());
        lines
    }
}
