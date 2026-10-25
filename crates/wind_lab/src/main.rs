//! Лаборатория: смотрим и крутим симуляцию в окне, поднимаем сервер
//! или подключаемся к серверу и играем за солдата.
//!
//! cargo run --release -p wind_lab

mod scene;
mod theme;
mod viewport;

use eframe::egui::{self, Color32, RichText};
use scene::Blast;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};
use viewport::Viewport;
use wind_net::{ClientStatus, DEFAULT_PORT, NetClient, NetServer, ReplicationConfig};
use wind_sim::{Event, PlayerInput, SimConfig, TickStats, Vec2, World};

/// Сколько последних тиков держим для графика.
const HISTORY: usize = 300;
/// Больше тиков за кадр не считаем, даже если не успеваем, — иначе окно зависнет.
const MAX_STEPS_PER_FRAME: u32 = 64;
/// Как часто клиент шлёт ввод, раз в секунду.
const INPUT_RATE: f32 = 30.0;
/// MSAA для 3D-вида. Совпадает в NativeOptions и в wind_render::install.
const MSAA: u32 = 4;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("The Wind Engine — Лаборатория")
            .with_inner_size([1600.0, 950.0]),
        // Для 3D-вида: буфер глубины и сглаживание, как в игре.
        depth_buffer: 32,
        multisampling: MSAA as u16,
        ..Default::default()
    };
    eframe::run_native("wind_lab", options, Box::new(|cc| Ok(Box::new(Lab::new(cc)))))
}

/// Параметры, которые меняются только перезапуском матча.
#[derive(Clone, PartialEq)]
struct MatchSettings {
    map_size: u32,
    soldiers_per_team: u32,
    capture_points: u32,
    seed: u64,
}

struct Lab {
    world: World,
    /// Последняя прочитанная версия `data/sim.ron`.
    base_config: SimConfig,
    config_path: PathBuf,
    config_mtime: Option<SystemTime>,
    last_config_check: Instant,
    settings: MatchSettings,
    paused: bool,
    speed: f32,
    accumulator: f32,
    history: VecDeque<TickStats>,
    selected: Option<u32>,
    viewport: Viewport,
    blasts: Vec<Blast>,
    status: (String, Color32),

    /// Сервер, поднятый прямо в лаборатории поверх локальной симуляции.
    server: Option<NetServer>,
    server_port: u16,
    /// Подключение к чужому (или своему) серверу. Пока оно есть, локальная симуляция стоит.
    client: Option<NetClient>,
    connect_addr: String,
    player_name: String,
    last_input: Instant,
    aim: Vec2,
    net_log: VecDeque<String>,
    /// Уже написали «в игре» для текущего подключения.
    announced: bool,
}

impl Lab {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        let config_path = find_config();
        let (base_config, status) = match SimConfig::load(&config_path) {
            Ok(c) => (c, (format!("Загружено: {}", config_path.display()), theme::TEXT_DIM)),
            Err(e) => (SimConfig::default(), (e, theme::ERROR)),
        };
        let settings = MatchSettings {
            map_size: base_config.map_size,
            soldiers_per_team: base_config.soldiers_per_team,
            capture_points: base_config.capture_points,
            seed: 1,
        };
        let world = World::new(with_settings(&base_config, &settings), settings.seed);
        let mut viewport = Viewport::new();
        if let Some(rs) = &cc.wgpu_render_state {
            let models = config_path.parent().map(|data| data.join("models/soldier"));
            wind_render::install(rs, MSAA, models.as_deref());
            viewport.has_3d = true;
        }
        let mut lab = Lab {
            world,
            config_mtime: mtime(&config_path),
            config_path,
            base_config,
            last_config_check: Instant::now(),
            settings,
            paused: false,
            speed: 1.0,
            accumulator: 0.0,
            history: VecDeque::with_capacity(HISTORY),
            selected: None,
            viewport,
            blasts: Vec::new(),
            status,
            server: None,
            server_port: DEFAULT_PORT,
            client: None,
            connect_addr: format!("127.0.0.1:{DEFAULT_PORT}"),
            player_name: "Игрок".into(),
            last_input: Instant::now(),
            aim: Vec2::ZERO,
            net_log: VecDeque::new(),
            announced: false,
        };
        // wind_lab --connect 127.0.0.1:7777 — сразу в игру; wind_lab --serve — сразу поднять сервер.
        let mut args = std::env::args().skip(1);
        while let Some(flag) = args.next() {
            match flag.as_str() {
                "--connect" => {
                    if let Some(addr) = args.next() {
                        lab.connect_addr = addr;
                    }
                    lab.connect();
                }
                "--serve" => lab.start_server(),
                "--3d" => lab.viewport.mode_3d = lab.viewport.has_3d,
                _ => {}
            }
        }
        lab
    }

    fn log(&mut self, line: String) {
        self.status = (line.clone(), theme::TEXT_DIM);
        self.net_log.push_front(line);
        self.net_log.truncate(8);
    }

    fn restart(&mut self) {
        // Клиенты сервера держатся за солдат старого мира — сервер тоже перезапускаем.
        let had_server = self.server.take().is_some();
        self.world = World::new(with_settings(&self.base_config, &self.settings), self.settings.seed);
        self.history.clear();
        self.selected = None;
        self.accumulator = 0.0;
        self.blasts.clear();
        self.viewport.reset();
        if had_server {
            self.start_server();
        }
    }

    fn start_server(&mut self) {
        match NetServer::bind(("0.0.0.0", self.server_port), ReplicationConfig::default()) {
            Ok(server) => {
                self.server = Some(server);
                self.log(format!("Сервер запущен на порту {}", self.server_port));
            }
            Err(e) => {
                self.status = (format!("Не удалось открыть порт {}: {e}", self.server_port), theme::ERROR);
            }
        }
    }

    fn connect(&mut self) {
        match NetClient::connect(&self.connect_addr, &self.player_name, None) {
            Ok(client) => {
                self.log(format!("Подключаюсь к {}…", client.server_addr()));
                self.client = Some(client);
                self.announced = false;
                self.viewport.reset_close(8.0);
            }
            Err(e) => self.status = (format!("{}: {e}", self.connect_addr), theme::ERROR),
        }
    }

    fn disconnect(&mut self) {
        if let Some(mut client) = self.client.take() {
            client.disconnect();
            self.log("Отключился от сервера".into());
        }
        self.viewport.reset();
        self.viewport.follow = false;
    }

    fn step(&mut self) {
        if let Some(server) = &mut self.server {
            server.poll(&mut self.world);
        }
        self.world.step();
        if let Some(server) = &mut self.server {
            server.send_snapshots(&self.world);
        }
        let lines: Vec<String> = self.server.iter_mut().flat_map(|s| s.log.drain(..)).collect();
        for line in lines {
            self.log(line);
        }
        for event in &self.world.events {
            if let Event::Explosion { pos, radius } = *event {
                self.blasts.push(Blast { pos, radius, age: 0.0 });
            }
        }
        if self.history.len() == HISTORY {
            self.history.pop_front();
        }
        self.history.push_back(self.world.stats.clone());
    }

    fn advance(&mut self, frame_dt: f32) {
        for b in &mut self.blasts {
            b.age += frame_dt;
        }
        self.blasts.retain(|b| b.age < 0.6);

        if let Some(client) = &mut self.client {
            client.poll();
            if let Some(w) = &mut client.world {
                w.advance_effects(frame_dt);
            }
            match client.status.clone() {
                ClientStatus::Connected if !self.announced => {
                    self.announced = true;
                    if let Some(w) = &client.world {
                        let line = format!(
                            "В игре: солдат #{}, команда «{}»",
                            w.welcome.soldier,
                            theme::TEAM_NAME[w.welcome.team as usize]
                        );
                        self.log(line);
                    }
                }
                ClientStatus::Rejected(why) => {
                    self.status = (format!("Сервер отказал: {why}"), theme::ERROR);
                    self.client = None;
                }
                ClientStatus::TimedOut => {
                    self.status = ("Сервер не отвечает — отключился".into(), theme::ERROR);
                    self.client = None;
                }
                _ => {}
            }
            // Своя симуляция на паузе, пока играем по сети. Но если мы же и сервер — она идёт.
            if self.server.is_none() {
                return;
            }
        }

        if self.paused {
            return;
        }
        let dt = self.world.dt();
        self.accumulator += frame_dt.min(0.25) * self.speed;
        let mut steps = 0;
        while self.accumulator >= dt && steps < MAX_STEPS_PER_FRAME {
            self.step();
            self.accumulator -= dt;
            steps += 1;
        }
        if steps == MAX_STEPS_PER_FRAME {
            self.accumulator = 0.0;
        }
    }

    /// WASD — идти, мышь — прицел, ЛКМ — огонь, G — граната.
    fn send_player_input(&mut self, ctx: &egui::Context, mouse_world: Option<Vec2>, primary_down: bool) {
        let Some(client) = &mut self.client else { return };
        if self.last_input.elapsed().as_secs_f32() < 1.0 / INPUT_RATE {
            return;
        }
        self.last_input = Instant::now();
        if let Some(m) = mouse_world {
            self.aim = m;
        }
        let typing = ctx.egui_wants_keyboard_input();
        let (mut move_dir, grenade) = ctx.input(|i| {
            let mut d = Vec2::ZERO;
            if !typing {
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
            }
            (d, !typing && i.key_down(egui::Key::G))
        });
        move_dir = move_dir.normalize_or_zero();
        client.send_input(PlayerInput { move_dir, aim: self.aim, fire: primary_down, grenade });
    }

    /// Раз в полсекунды смотрим, не изменился ли файл с параметрами.
    fn hot_reload(&mut self) {
        if self.last_config_check.elapsed().as_secs_f32() < 0.5 {
            return;
        }
        self.last_config_check = Instant::now();
        let current = mtime(&self.config_path);
        if current == self.config_mtime {
            return;
        }
        self.config_mtime = current;
        self.reload_config();
    }

    fn reload_config(&mut self) {
        match SimConfig::load(&self.config_path) {
            Ok(config) => {
                self.base_config = config;
                // Параметры матча (карта, число солдат) остаются, остальное — сразу в бой.
                let live = MatchSettings {
                    map_size: self.world.config.map_size,
                    soldiers_per_team: self.world.config.soldiers_per_team,
                    capture_points: self.world.config.capture_points,
                    seed: self.world.seed,
                };
                self.world.config = with_settings(&self.base_config, &live);
                self.status = (format!("Перезагружено: {}", self.config_path.display()), theme::TEXT_DIM);
            }
            Err(e) => self.status = (e, theme::ERROR),
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("topbar")
            .frame(egui::Frame::new().fill(theme::TOPBAR).inner_margin(egui::Margin::same(4)))
            .show(ui, |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    ui.label(RichText::new("  The Wind Engine  ").strong());
                    ui.menu_button("Файл", |ui| {
                        if ui.button("Перечитать data/sim.ron").clicked() {
                            self.reload_config();
                        }
                        if ui.button("Выход").clicked() {
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                    ui.menu_button("Симуляция", |ui| {
                        let label = if self.paused { "Пуск  (Пробел)" } else { "Пауза  (Пробел)" };
                        if ui.button(label).clicked() {
                            self.paused = !self.paused;
                        }
                        if ui.button("Шаг на тик  (→)").clicked() {
                            self.step();
                        }
                        if ui.button("Перезапустить матч").clicked() {
                            self.restart();
                        }
                    });
                    ui.menu_button("Вид", |ui| {
                        if self.viewport.has_3d {
                            ui.checkbox(&mut self.viewport.mode_3d, "3D-вид");
                        }
                        let o = &mut self.viewport.overlays;
                        ui.checkbox(&mut o.grid, "Сетка");
                        ui.checkbox(&mut o.bullets, "Пули и гранаты");
                        ui.checkbox(&mut o.vision, "Обзор выделенного");
                        ui.checkbox(&mut o.flow, "Поле путей выделенного");
                        ui.checkbox(&mut self.viewport.follow, "Следовать за выделенным");
                    });
                    ui.add_space(24.0);
                    // Вкладки рабочих пространств, как Layout / Modeling в Blender.
                    let _ = ui.selectable_label(self.client.is_none(), "Лаборатория");
                    let _ = ui.selectable_label(self.client.is_some(), "Игра по сети");
                });
            });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status")
            .frame(egui::Frame::new().fill(theme::TOPBAR).inner_margin(egui::Margin::same(4)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&self.status.0).color(self.status.1).small());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let fps = 1.0 / ui.input(|i| i.stable_dt).max(1e-4);
                        let text = match &self.client {
                            Some(c) => format!(
                                "Сеть: к нам {:.1} КБ/с · от нас {:.2} КБ/с · пинг {:.0} мс · {:.0} FPS",
                                c.traffic.recv_per_sec / 1024.0,
                                c.traffic.sent_per_sec / 1024.0,
                                c.rtt_ms,
                                fps
                            ),
                            None => format!(
                                "Тик {} · {:.1} с · {:.2} мс/тик · {:.0} FPS",
                                self.world.tick,
                                self.world.tick as f32 * self.world.dt(),
                                self.world.stats.total_ms,
                                fps
                            ),
                        };
                        ui.label(RichText::new(text).small().color(theme::TEXT_DIM));
                    });
                });
            });
    }

    fn outliner(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("outliner")
            .resizable(true)
            .default_size(230.0)
            .frame(egui::Frame::new().fill(theme::OUTLINER).inner_margin(egui::Margin::same(6)))
            .show(ui, |ui| {
                ui.label(RichText::new("Сцена").strong());
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if self.client.is_some() {
                        self.remote_outliner(ui);
                    } else {
                        self.local_outliner(ui);
                    }
                });
            });
    }

    fn local_outliner(&mut self, ui: &mut egui::Ui) {
        for team in 0..2 {
            let total = self.world.soldiers.iter().filter(|s| s.team == team as u8).count();
            let alive = self.world.stats.alive[team];
            let title = RichText::new(format!(
                "{}  {alive}/{total} · убийств {}",
                theme::TEAM_NAME[team],
                self.world.score[team]
            ))
            .color(theme::TEAM[team]);
            egui::CollapsingHeader::new(title).id_salt(("team", team)).default_open(true).show(ui, |ui| {
                // Лучшие бойцы команды — клик выделяет.
                let mut best: Vec<(usize, u32)> = self
                    .world
                    .soldiers
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| s.team == team as u8)
                    .map(|(i, s)| (i, s.kills))
                    .collect();
                best.sort_by_key(|&(_, kills)| std::cmp::Reverse(kills));
                for (i, kills) in best.into_iter().take(8) {
                    let selected = self.selected == Some(i as u32);
                    if ui.selectable_label(selected, format!("Солдат #{i} — {kills}")).clicked() {
                        self.selected = Some(i as u32);
                        self.viewport.focus(self.world.soldiers[i].pos);
                    }
                }
            });
        }
        if let Some(server) = &self.server {
            egui::CollapsingHeader::new("Игроки на сервере").default_open(true).show(ui, |ui| {
                if server.clients.is_empty() {
                    ui.label(RichText::new("пока никого").color(theme::TEXT_DIM));
                }
                for c in &server.clients {
                    let label = RichText::new(format!("{} — солдат #{}", c.name, c.replica.soldier)).color(theme::TEAM[c.team as usize]);
                    if ui.selectable_label(self.selected == Some(c.replica.soldier), label).clicked() {
                        self.selected = Some(c.replica.soldier);
                        self.viewport.focus(self.world.soldiers[c.replica.soldier as usize].pos);
                    }
                }
            });
        }
        egui::CollapsingHeader::new("Точки захвата").default_open(true).show(ui, |ui| {
            for (i, p) in self.world.points.iter().enumerate() {
                if point_label(ui, i, p.owner).clicked() {
                    self.viewport.focus(p.pos);
                }
            }
        });
    }

    fn remote_outliner(&mut self, ui: &mut egui::Ui) {
        let Some(w) = self.client.as_ref().and_then(|c| c.world.as_ref()) else {
            ui.label(RichText::new("Ждём ответа сервера…").color(theme::TEXT_DIM));
            return;
        };
        for team in 0..2 {
            let mine = if w.welcome.team == team as u8 { " (мы)" } else { "" };
            ui.label(
                RichText::new(format!("{}{mine} · убийств {}", theme::TEAM_NAME[team], w.score[team]))
                    .color(theme::TEAM[team]),
            );
        }
        let visible = w.soldiers.iter().flatten().count();
        ui.label(RichText::new(format!("Видно солдат: {visible}")).color(theme::TEXT_DIM));
        egui::CollapsingHeader::new("Точки захвата").default_open(true).show(ui, |ui| {
            for (i, p) in w.points.iter().enumerate() {
                point_label(ui, i, p.owner);
            }
        });
    }

    fn properties(&mut self, ui: &mut egui::Ui) {
        egui::Panel::right("properties")
            .resizable(true)
            .default_size(300.0)
            .frame(egui::Frame::new().fill(theme::REGION).inner_margin(egui::Margin::same(6)))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if self.client.is_some() {
                        theme::panel(ui, "Подключение", true, |ui| self.client_panel(ui));
                        theme::panel(ui, "Управление", true, controls_panel);
                    } else {
                        theme::panel(ui, "Симуляция", true, |ui| self.simulation_panel(ui));
                        theme::panel(ui, "Матч", true, |ui| self.match_panel(ui));
                        theme::panel(ui, "Сеть", true, |ui| self.network_panel(ui));
                        theme::panel(ui, "Производительность", true, |ui| self.performance_panel(ui));
                        theme::panel(ui, "Выделение", true, |ui| self.selection_panel(ui));
                    }
                });
            });
    }

    fn simulation_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let label = if self.paused { "▶ Пуск" } else { "⏸ Пауза" };
            if ui.button(label).clicked() {
                self.paused = !self.paused;
            }
            if ui.add_enabled(self.paused, egui::Button::new("Шаг")).clicked() {
                self.step();
            }
        });
        theme::props(ui, "sim_props", |ui| {
            theme::label(ui, "Скорость");
            egui::ComboBox::from_id_salt("speed")
                .selected_text(format!("×{}", self.speed))
                .show_ui(ui, |ui| {
                    for s in [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0] {
                        ui.selectable_value(&mut self.speed, s, format!("×{s}"));
                    }
                });
            ui.end_row();
            theme::label(ui, "Тиков в секунду");
            ui.label(self.world.config.tick_rate.to_string());
            ui.end_row();
            theme::label(ui, "ИИ думает");
            ui.label(format!("раз в {} тиков", self.world.config.think_interval));
            ui.end_row();
        });
        ui.label(
            RichText::new("Оружие, ИИ и гранаты правятся в data/sim.ron — изменения подхватываются на лету.")
                .small()
                .color(theme::TEXT_DIM),
        );
    }

    fn match_panel(&mut self, ui: &mut egui::Ui) {
        theme::props(ui, "match_props", |ui| {
            theme::label(ui, "Размер карты");
            egui::ComboBox::from_id_salt("map_size")
                .selected_text(format!("{0}×{0} м", self.settings.map_size))
                .show_ui(ui, |ui| {
                    for m in [256, 512, 1024, 2048] {
                        ui.selectable_value(&mut self.settings.map_size, m, format!("{m}×{m} м"));
                    }
                });
            ui.end_row();
            theme::label(ui, "Солдат в команде");
            ui.add(egui::DragValue::new(&mut self.settings.soldiers_per_team).range(1..=10_000).speed(10));
            ui.end_row();
            theme::label(ui, "Точек захвата");
            ui.add(egui::DragValue::new(&mut self.settings.capture_points).range(1..=26));
            ui.end_row();
            theme::label(ui, "Seed");
            ui.add(egui::DragValue::new(&mut self.settings.seed));
            ui.end_row();
        });
        let changed = self.settings.map_size != self.world.config.map_size
            || self.settings.soldiers_per_team != self.world.config.soldiers_per_team
            || self.settings.capture_points != self.world.config.capture_points
            || self.settings.seed != self.world.seed;
        let button = egui::Button::new(if changed { "Перезапустить с новыми параметрами" } else { "Перезапустить матч" });
        if ui.add_sized([ui.available_width(), 22.0], button).clicked() {
            self.restart();
        }
    }

    fn network_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("Свой сервер").strong());
        match &self.server {
            None => {
                theme::props(ui, "server_props", |ui| {
                    theme::label(ui, "Порт");
                    ui.add(egui::DragValue::new(&mut self.server_port).range(1024..=65535));
                    ui.end_row();
                });
                if ui.add_sized([ui.available_width(), 22.0], egui::Button::new("Запустить сервер")).clicked() {
                    self.start_server();
                }
            }
            Some(server) => {
                let down: f32 = server.clients.iter().map(|c| c.traffic.sent_per_sec).sum();
                let n = server.clients.len();
                theme::props(ui, "server_stats", |ui| {
                    theme::label(ui, "Порт");
                    ui.label(self.server_port.to_string());
                    ui.end_row();
                    theme::label(ui, "Игроков");
                    ui.label(format!("{n} / {}", server.max_clients));
                    ui.end_row();
                    theme::label(ui, "На игрока");
                    ui.label(format!("{:.1} КБ/с", down / n.max(1) as f32 / 1024.0));
                    ui.end_row();
                    theme::label(ui, "Отдаёт всего");
                    ui.label(format!("{:.0} КБ/с", server.traffic.sent_per_sec / 1024.0));
                    ui.end_row();
                });
                if ui.add_sized([ui.available_width(), 22.0], egui::Button::new("Остановить сервер")).clicked() {
                    // Без сервера игроки пропадут — отдаём их солдат обратно ИИ.
                    for c in &server.clients {
                        self.world.release_control(c.replica.soldier);
                    }
                    self.server = None;
                    self.log("Сервер остановлен".into());
                }
            }
        }
        ui.add_space(6.0);
        ui.label(RichText::new("Подключиться").strong());
        theme::props(ui, "connect_props", |ui| {
            theme::label(ui, "Адрес");
            ui.add(egui::TextEdit::singleline(&mut self.connect_addr).desired_width(150.0));
            ui.end_row();
            theme::label(ui, "Имя");
            ui.add(egui::TextEdit::singleline(&mut self.player_name).desired_width(150.0));
            ui.end_row();
        });
        if ui.add_sized([ui.available_width(), 22.0], egui::Button::new("Подключиться и играть")).clicked() {
            self.connect();
        }
        for line in &self.net_log {
            ui.label(RichText::new(line).small().color(theme::TEXT_DIM));
        }
    }

    fn client_panel(&mut self, ui: &mut egui::Ui) {
        let Some(client) = &mut self.client else { return };
        let status = match &client.status {
            ClientStatus::Connecting => "подключаюсь…".to_string(),
            ClientStatus::Connected => "в игре".to_string(),
            ClientStatus::Rejected(why) => format!("отказ: {why}"),
            ClientStatus::TimedOut => "сервер не отвечает".to_string(),
        };
        theme::props(ui, "client_props", |ui| {
            theme::label(ui, "Сервер");
            ui.label(client.server_addr().to_string());
            ui.end_row();
            theme::label(ui, "Состояние");
            ui.label(status);
            ui.end_row();
            theme::label(ui, "Пинг");
            ui.label(format!("{:.0} мс", client.rtt_ms));
            ui.end_row();
            theme::label(ui, "К нам");
            ui.label(format!("{:.1} КБ/с", client.traffic.recv_per_sec / 1024.0));
            ui.end_row();
            theme::label(ui, "От нас");
            ui.label(format!("{:.2} КБ/с", client.traffic.sent_per_sec / 1024.0));
            ui.end_row();
            theme::label(ui, "Пакет до");
            ui.label(format!("{} Б", client.traffic.max_recv_packet));
            ui.end_row();
            theme::label(ui, "Не разобрано");
            ui.label(client.decode_errors.to_string());
            ui.end_row();
            theme::label(ui, "Терять пакеты");
            ui.add(egui::Slider::new(&mut client.simulated_loss, 0.0..=0.5).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
            ui.end_row();
        });
        ui.label(
            RichText::new("«Терять пакеты» нарочно выбрасывает часть входящих снимков — проверить, как игра переживает плохую сеть.")
                .small()
                .color(theme::TEXT_DIM),
        );
        if ui.add_sized([ui.available_width(), 22.0], egui::Button::new("Отключиться")).clicked() {
            self.disconnect();
        }
    }

    fn performance_panel(&mut self, ui: &mut egui::Ui) {
        let n = self.history.len().max(1) as f32;
        let avg = |f: fn(&TickStats) -> f32| self.history.iter().map(f).sum::<f32>() / n;
        let budget = 1000.0 / self.world.config.tick_rate as f32;
        let total = avg(|s| s.total_ms);
        let worst = self.history.iter().map(|s| s.total_ms).fold(0.0, f32::max);

        theme::props(ui, "perf_props", |ui| {
            theme::label(ui, "Тик, среднее");
            ui.label(format!("{total:.2} мс из {budget:.0}"));
            ui.end_row();
            theme::label(ui, "Тик, худший");
            ui.label(format!("{worst:.2} мс"));
            ui.end_row();
            theme::label(ui, "Пуль в полёте");
            ui.label(self.world.stats.bullets.to_string());
            ui.end_row();
            theme::label(ui, "Гранат");
            ui.label(self.world.stats.grenades.to_string());
            ui.end_row();
        });
        ui.add_space(4.0);

        // Полоски по фазам: какая часть тика на что уходит.
        type Phase = (&'static str, fn(&TickStats) -> f32);
        let phases: [Phase; 7] = [
            ("Сетка", |s| s.grid_ms),
            ("Пути", |s| s.nav_ms),
            ("ИИ", |s| s.think_ms),
            ("Движение", |s| s.move_ms),
            ("Стрельба", |s| s.fire_ms),
            ("Снаряды", |s| s.projectiles_ms),
            ("Точки", |s| s.capture_ms),
        ];
        let max = phases.iter().map(|(_, f)| avg(*f)).fold(0.001, f32::max);
        for (name, f) in phases {
            let v = avg(f);
            ui.horizontal(|ui| {
                ui.add_sized([70.0, 16.0], egui::Label::new(RichText::new(name).small().color(theme::TEXT_DIM)));
                let width = ui.available_width() - 60.0;
                let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 8.0), egui::Sense::hover());
                ui.painter().rect_filled(rect, 2, theme::FIELD);
                let mut bar = rect;
                bar.set_width(rect.width() * v / max);
                ui.painter().rect_filled(bar, 2, theme::ACCENT);
                ui.label(RichText::new(format!("{v:.3}")).small());
            });
        }
        ui.add_space(6.0);

        // График времени тика с линией бюджета.
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 70.0), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 3, theme::FIELD);
        let scale = worst.max(budget * 0.05) * 1.2;
        let y_of = |ms: f32| rect.bottom() - (ms / scale).min(1.0) * rect.height();
        if budget < scale {
            let y = y_of(budget);
            painter.line_segment([egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)], egui::Stroke::new(1.0, theme::ERROR));
        }
        let points: Vec<egui::Pos2> = self
            .history
            .iter()
            .enumerate()
            .map(|(i, s)| egui::pos2(rect.left() + rect.width() * i as f32 / HISTORY as f32, y_of(s.total_ms)))
            .collect();
        painter.add(egui::Shape::line(points, egui::Stroke::new(1.0, theme::ACCENT)));
        painter.text(
            rect.left_top() + egui::vec2(4.0, 2.0),
            egui::Align2::LEFT_TOP,
            format!("{scale:.1} мс"),
            egui::FontId::proportional(10.0),
            theme::TEXT_DIM,
        );
    }

    fn selection_panel(&mut self, ui: &mut egui::Ui) {
        let Some(i) = self.selected else {
            ui.label(RichText::new("Кликни по солдату во вьюпорте.").color(theme::TEXT_DIM));
            return;
        };
        let s = &self.world.soldiers[i as usize];
        theme::props(ui, "sel_props", |ui| {
            theme::label(ui, "Солдат");
            ui.label(RichText::new(format!("#{i} · {}", theme::TEAM_NAME[s.team as usize])).color(theme::TEAM[s.team as usize]));
            ui.end_row();
            theme::label(ui, "Управляет");
            ui.label(match s.controller {
                wind_sim::Controller::Ai => "ИИ",
                wind_sim::Controller::Player(_) => "игрок по сети",
            });
            ui.end_row();
            theme::label(ui, "Состояние");
            if s.alive {
                ui.label(format!("жив, здоровье {:.0}%", s.health * 100.0));
            } else {
                ui.label(format!("мёртв, появится через {:.1} с", s.respawn_timer.max(0.0)));
            }
            ui.end_row();
            theme::label(ui, "Цель");
            ui.label(s.target.map_or("—".into(), |t| format!("солдат #{t}")));
            ui.end_row();
            theme::label(ui, "Идёт к");
            ui.label(match s.objective {
                o if (o as usize) < self.world.points.len() => format!("точке {}", (b'A' + o as u8) as char),
                _ => "—".into(),
            });
            ui.end_row();
            theme::label(ui, "Патроны");
            if s.reload_timer > 0.0 {
                ui.label("перезарядка…");
            } else {
                ui.label(format!("{} / {}", s.ammo, self.world.config.weapon.magazine_size));
            }
            ui.end_row();
            theme::label(ui, "Гранаты");
            ui.label(s.grenades.to_string());
            ui.end_row();
            theme::label(ui, "Убийств");
            ui.label(s.kills.to_string());
            ui.end_row();
        });
        ui.checkbox(&mut self.viewport.follow, "Следовать камерой");
    }
}

fn controls_panel(ui: &mut egui::Ui) {
    theme::props(ui, "controls", |ui| {
        for (key, what) in [
            ("W A S D", "идти"),
            ("Мышь", "прицел"),
            ("ЛКМ", "огонь"),
            ("G", "граната"),
            ("Колесо", "масштаб"),
            ("СКМ / ПКМ", "двигать вид"),
        ] {
            theme::label(ui, key);
            ui.label(what);
            ui.end_row();
        }
    });
}

fn point_label(ui: &mut egui::Ui, index: usize, owner: Option<u8>) -> egui::Response {
    let (name, color) = match owner {
        Some(t) => (theme::TEAM_NAME[t as usize], theme::TEAM[t as usize]),
        None => ("ничья", theme::TEXT_DIM),
    };
    let letter = (b'A' + index as u8) as char;
    ui.selectable_label(false, RichText::new(format!("Точка {letter} — {name}")).color(color))
}

impl eframe::App for Lab {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.hot_reload();
        let typing = ui.ctx().egui_wants_keyboard_input();
        let (frame_dt, space, right) = ui.input(|i| {
            (i.stable_dt, i.key_pressed(egui::Key::Space), i.key_pressed(egui::Key::ArrowRight))
        });
        if self.client.is_none() && !typing {
            if space {
                self.paused = !self.paused;
            }
            if right && self.paused {
                self.step();
            }
        }
        self.advance(frame_dt);

        self.top_bar(ui);
        self.status_bar(ui);
        self.outliner(ui);
        self.properties(ui);
        let ctx = ui.ctx().clone();
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let playing = self.client.is_some();
            let scene = match &self.client {
                Some(client) => scene::remote(client),
                None => Some(scene::local(&self.world, self.selected, &self.blasts)),
            };
            let Some(scene) = scene else {
                ui.centered_and_justified(|ui| ui.label("Ждём ответа сервера…"));
                return;
            };
            let out = self.viewport.show(ui, &scene, playing);
            if let Some(pick) = out.picked {
                self.selected = pick;
            }
            if playing {
                self.viewport.follow = true;
                self.send_player_input(&ctx, out.mouse_world, out.primary_down);
            }
        });
        ui.ctx().request_repaint();
    }
}

fn with_settings(base: &SimConfig, s: &MatchSettings) -> SimConfig {
    let mut config = base.clone();
    config.map_size = s.map_size;
    config.soldiers_per_team = s.soldiers_per_team;
    config.capture_points = s.capture_points;
    config
}

/// `data/sim.ron` ищем от текущей папки, а если запустили не из корня — от корня проекта.
fn find_config() -> PathBuf {
    let local = PathBuf::from("data/sim.ron");
    if local.exists() {
        return local;
    }
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/sim.ron")
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}
