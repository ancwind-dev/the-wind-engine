//! Лаборатория — рабочее место движка: симуляция боя, сеть, редактор карт и студия
//! персонажей. Открывается для проекта с главного экрана (см. `main.rs`).

use crate::i18n::{f, t};
use eframe::egui::{self, Color32, RichText};
use scene::Blast;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use crate::project::Project;
use crate::{editor_ui, icons, scene, studio, theme, viewport};
use std::time::{Instant, SystemTime};
use editor_ui::MapWorkspace;
use icons::Icon;
use studio::Studio;
use viewport::Viewport;
use wind_sim::mapfile::MapFile;
use wind_net::{ClientStatus, DEFAULT_PORT, NetClient, NetServer, ReplicationConfig};
use wind_sim::{Event, PlayerInput, SimConfig, TickStats, Vec2, World};

/// Сколько последних тиков держим для графика.
const HISTORY: usize = 300;
/// Больше тиков за кадр не считаем, даже если не успеваем, — иначе окно зависнет.
const MAX_STEPS_PER_FRAME: u32 = 64;
/// Как часто клиент шлёт ввод, раз в секунду.
const INPUT_RATE: f32 = 30.0;
/// MSAA для 3D-вида. Совпадает в NativeOptions и в wind_render::install.
pub const MSAA: u32 = 4;

/// Параметры, которые меняются только перезапуском матча.
#[derive(Clone, PartialEq)]
struct MatchSettings {
    map_size: u32,
    soldiers_per_team: u32,
    capture_points: u32,
    seed: u64,
}

pub struct Lab {
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
    /// Пароль сервера, к которому подключаемся.
    connect_password: String,
    last_input: Instant,
    aim: Vec2,
    net_log: VecDeque<String>,
    /// Уже написали «в игре» для текущего подключения.
    announced: bool,

    workspace: Workspace,
    map_ws: MapWorkspace,
    /// Свой вьюпорт у редактора — камера не прыгает при переключении вкладок.
    editor_view: Viewport,
    /// Солдаты «точно как RWR» (по умолчанию) или «как было».
    rwr_exact: bool,
    map_generation: u64,
    /// Карта из файла (или из редактора), на которой идёт симуляция. `None` — процедурная.
    map_file: Option<MapFile>,
    studio: Studio,
}

/// Что лаборатория просит у оболочки движка (кнопки шапки).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LabAction {
    /// На главный экран проекта.
    Home,
    /// К окну выбора проекта.
    ChangeProject,
    /// Открыть настройки движка.
    Settings,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Workspace {
    Simulation,
    Map,
    Studio,
}

impl Lab {
    /// Лаборатория для проекта. `render_state` нет — без 3D-вида (рисуем сверху).
    pub fn new(render_state: Option<&eframe::egui_wgpu::RenderState>, project: &Project) -> Self {
        let config_path = project.config_path();
        let (base_config, status) = match SimConfig::load(&config_path) {
            Ok(c) => (c, (f("Загружено: {}", &[&config_path.display()]), theme::TEXT_DIM)),
            Err(e) => (SimConfig::default(), (e, theme::ERROR)),
        };
        let settings = MatchSettings {
            map_size: base_config.map_size,
            soldiers_per_team: base_config.soldiers_per_team,
            capture_points: base_config.capture_points,
            seed: 1,
        };
        let world = World::new(with_settings(&base_config, &settings), settings.seed);
        let data = project.data_dir();
        let mut viewport = Viewport::new(&data);
        let mut editor_view = Viewport::new(&data);
        let mut characters = None;
        if let Some(rs) = render_state {
            let installed = wind_render::install(rs, MSAA, Some(&data));
            viewport.set_characters(installed.characters.clone());
            editor_view.set_characters(installed.characters.clone());
            characters = installed.characters;
            viewport.has_3d = true;
            editor_view.has_3d = true;
        }
        let studio = Studio::new(characters, render_state.is_some(), project.data_dir());
        let map_ws = MapWorkspace::new(project.maps_dir(), &base_config);
        Lab {
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
            player_name: t("Игрок").into(),
            connect_password: String::new(),
            last_input: Instant::now(),
            aim: Vec2::ZERO,
            net_log: VecDeque::new(),
            announced: false,
            workspace: Workspace::Simulation,
            map_ws,
            editor_view,
            map_generation: 0,
            map_file: None,
            studio,
            rwr_exact: true,
        }
    }

    /// Ключи командной строки: `--connect 127.0.0.1:7777` — сразу в игру, `--serve` — поднять
    /// сервер, `--editor` / `--studio` — сразу в редактор карт или студию и т. д.
    pub fn apply_args(&mut self, args: &[String]) {
        let mut args = args.iter().cloned();
        while let Some(flag) = args.next() {
            match flag.as_str() {
                "--connect" => {
                    if let Some(addr) = args.next() {
                        self.connect_addr = addr;
                    }
                    self.connect();
                }
                "--serve" => self.start_server(),
                "--3d" => {
                    self.viewport.mode_3d = self.viewport.has_3d;
                    self.editor_view.mode_3d = self.editor_view.has_3d;
                }
                "--open" => {
                    if let Some(name) = args.next() {
                        self.map_ws.open(&name);
                        self.set_workspace(Workspace::Map);
                    }
                }
                // wind_lab --play-map руины — бой ИИ на карте проекта.
                "--play-map" => {
                    if let Some(name) = args.next() {
                        match MapFile::load(&self.map_ws.maps_dir().join(&name)) {
                            Ok(file) => {
                                self.map_file = Some(file);
                                self.restart();
                            }
                            Err(e) => self.log(format!("{name}: {e}")),
                        }
                    }
                }
                "--editor" => self.set_workspace(Workspace::Map),
                "--studio" => self.set_workspace(Workspace::Studio),
                // wind_lab --3d --follow 5 — выделить солдата №5 и следовать за ним вблизи.
                "--follow" => {
                    if let Some(n) = args.next().and_then(|v| v.parse().ok()) {
                        self.selected = Some(n);
                        self.viewport.reset_close(6.0);
                    }
                }
                _ => {}
            }
        }
    }

    fn log(&mut self, line: String) {
        self.status = (line.clone(), theme::TEXT_DIM);
        self.net_log.push_front(line);
        self.net_log.truncate(8);
    }

    fn restart(&mut self) {
        // Клиенты сервера держатся за солдат старого мира — сервер тоже перезапускаем.
        let had_server = self.server.take().is_some();
        let config = with_settings(&self.base_config, &self.settings);
        self.world = match &self.map_file {
            Some(file) => World::from_file(config, self.settings.seed, file.clone()),
            None => World::new(config, self.settings.seed),
        };
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
                self.log(f("Сервер запущен на порту {}", &[&self.server_port]));
            }
            Err(e) => {
                self.status = (f("Не удалось открыть порт {}: {}", &[&self.server_port, &e]), theme::ERROR);
            }
        }
    }

    fn connect(&mut self) {
        match NetClient::connect(&self.connect_addr, &self.player_name, None, &self.connect_password) {
            Ok(mut client) => {
                client.maps_dir = self.map_ws.maps_dir().to_path_buf();
                self.log(f("Подключаюсь к {}…", &[&client.server_addr()]));
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
            self.log(t("Отключился от сервера").into());
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
                        let line = f("В игре: солдат #{}, команда «{}»", &[&w.welcome.soldier, &theme::team_name(w.welcome.team as usize)]);
                        self.log(line);
                    }
                }
                ClientStatus::Rejected(why) => {
                    self.status = (f("Сервер отказал: {}", &[&why]), theme::ERROR);
                    self.client = None;
                }
                ClientStatus::TimedOut => {
                    self.status = (t("Сервер не отвечает — отключился").into(), theme::ERROR);
                    self.client = None;
                }
                _ => {}
            }
            // Своя симуляция на паузе, пока играем по сети. Но если мы же и сервер — она идёт.
            if self.server.is_none() {
                return;
            }
        }

        // В редакторе и студии симуляция стоит — если только на ней не играют по сети.
        let editing = self.workspace != Workspace::Simulation && self.server.is_none();
        if self.paused || editing {
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
        let (mut move_dir, grenade, reload, walk) = ctx.input(|i| {
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
            (d, !typing && i.key_down(egui::Key::G), !typing && i.key_down(egui::Key::R), !typing && i.modifiers.shift)
        });
        move_dir = move_dir.normalize_or_zero();
        client.send_input(PlayerInput { move_dir, aim: self.aim, fire: primary_down, grenade, reload, walk, ..Default::default() });
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
                self.status = (f("Перезагружено: {}", &[&self.config_path.display()]), theme::TEXT_DIM);
            }
            Err(e) => self.status = (e, theme::ERROR),
        }
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) -> Option<LabAction> {
        let mut action = None;
        egui::Panel::top("topbar")
            .frame(egui::Frame::new().fill(theme::TOPBAR).inner_margin(egui::Margin::same(4)))
            .show(ui, |ui| {
                egui::MenuBar::new().ui(ui, |ui| {
                    // Логотип — на главный экран, как кнопка «домой».
                    if theme::logo_button(ui).on_hover_text(t("Главный экран")).clicked() {
                        action = Some(LabAction::Home);
                    }
                    ui.menu_button(t("Файл"), |ui| {
                        if icons::button(ui, Icon::Home, t("Главный экран")).clicked() {
                            action = Some(LabAction::Home);
                        }
                        if icons::button(ui, Icon::Open, t("Сменить проект…")).clicked() {
                            action = Some(LabAction::ChangeProject);
                        }
                        ui.separator();
                        if icons::button(ui, Icon::Restart, t("Перечитать sim.ron")).clicked() {
                            self.reload_config();
                        }
                        if icons::button(ui, Icon::Gear, t("Настройки…")).clicked() {
                            action = Some(LabAction::Settings);
                        }
                        ui.separator();
                        if ui.button(t("Выход")).clicked() {
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                    ui.menu_button(t("Симуляция"), |ui| {
                        let (icon, label) = if self.paused { (Icon::Play, t("Пуск  (Пробел)")) } else { (Icon::Pause, t("Пауза  (Пробел)")) };
                        if icons::button(ui, icon, label).clicked() {
                            self.paused = !self.paused;
                        }
                        if icons::button(ui, Icon::Step, t("Шаг на тик  (→)")).clicked() {
                            self.step();
                        }
                        if icons::button(ui, Icon::Restart, t("Перезапустить матч")).clicked() {
                            self.restart();
                        }
                    });
                    ui.menu_button(t("Вид"), |ui| {
                        if self.viewport.has_3d {
                            ui.checkbox(&mut self.viewport.mode_3d, t("3D-вид"));
                        }
                        let o = &mut self.viewport.overlays;
                        ui.checkbox(&mut o.grid, t("Сетка"));
                        ui.checkbox(&mut o.bullets, t("Пули и гранаты"));
                        ui.checkbox(&mut o.vision, t("Обзор выделенного"));
                        ui.checkbox(&mut o.flow, t("Поле путей выделенного"));
                        ui.checkbox(&mut self.viewport.follow, t("Следовать за выделенным"));
                        ui.separator();
                        if ui.checkbox(&mut self.rwr_exact, t("Солдаты точно как RWR")).on_hover_text(t("Точки в пикселях экрана, оружие без света. Выключить — как было до 2026-10-28")).changed() {
                            for v in [&mut self.viewport, &mut self.editor_view, &mut self.studio.view] {
                                v.set_rwr_exact(self.rwr_exact);
                            }
                        }
                    });
                    ui.add_space(24.0);
                    // Вкладки рабочих пространств, как Layout / Modeling в Blender.
                    if self.client.is_some() {
                        let _ = icons::selectable(ui, true, Icon::Network, t("Игра по сети"), None);
                    } else {
                        if icons::selectable(ui, self.workspace == Workspace::Simulation, Icon::Simulation, t("Лаборатория"), None).clicked() {
                            self.set_workspace(Workspace::Simulation);
                        }
                        if icons::selectable(ui, self.workspace == Workspace::Map, Icon::MapEditor, t("Редактор карт"), None).clicked() {
                            self.set_workspace(Workspace::Map);
                        }
                        if icons::selectable(ui, self.workspace == Workspace::Studio, Icon::Studio, t("Студия персонажей"), None).clicked() {
                            self.set_workspace(Workspace::Studio);
                        }
                    }
                });
            });
        action
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        egui::Panel::bottom("status")
            .frame(egui::Frame::new().fill(theme::TOPBAR).inner_margin(egui::Margin::same(4)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    if self.client.is_none() && self.workspace == Workspace::Map {
                        // В редакторе внизу — подсказки по инструменту, как в строке состояния Blender.
                        ui.label(RichText::new(self.map_ws.hint()).small().color(theme::TEXT_DIM));
                        if !self.map_ws.message.0.is_empty() {
                            ui.label(RichText::new(format!("· {}", self.map_ws.message.0)).small().color(self.map_ws.message.1));
                        }
                        return;
                    }
                    ui.label(RichText::new(&self.status.0).color(self.status.1).small());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let fps = 1.0 / ui.input(|i| i.stable_dt).max(1e-4);
                        let text = match &self.client {
                            Some(c) => f("Сеть: к нам {:.1} КБ/с · от нас {:.2} КБ/с · пинг {:.0} мс · {:.0} FPS", &[&(c.traffic.recv_per_sec / 1024.0), &(c.traffic.sent_per_sec / 1024.0), &c.rtt_ms, &fps]),
                            None => f("Тик {} · {:.1} с · {:.2} мс/тик · {:.0} FPS", &[&self.world.tick, &(self.world.tick as f32 * self.world.dt()), &self.world.stats.total_ms, &fps]),
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
                ui.label(RichText::new(t("Сцена")).strong());
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
            let title = f("{}  {}/{} · убийств {}", &[&theme::team_name(team), &alive, &total, &self.world.score[team]]);
            icons::collapsing(ui, Icon::Team, &title, theme::TEAM[team], ("team", team), |ui| {
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
                    if icons::selectable(ui, selected, Icon::Soldier, &f("Солдат #{} — {}", &[&i, &kills]), None).clicked() {
                        self.selected = Some(i as u32);
                        self.viewport.focus(self.world.soldiers[i].pos);
                    }
                }
            });
        }
        if let Some(server) = &self.server {
            egui::CollapsingHeader::new(t("Игроки на сервере")).default_open(true).show(ui, |ui| {
                if server.clients.is_empty() {
                    ui.label(RichText::new(t("пока никого")).color(theme::TEXT_DIM));
                }
                for c in &server.clients {
                    let label = RichText::new(f("{} — солдат #{}", &[&c.name, &c.replica.soldier])).color(theme::TEAM[c.team as usize]);
                    if ui.selectable_label(self.selected == Some(c.replica.soldier), label).clicked() {
                        self.selected = Some(c.replica.soldier);
                        self.viewport.focus(self.world.soldiers[c.replica.soldier as usize].pos);
                    }
                }
            });
        }
        icons::collapsing(ui, Icon::Flag, t("Точки захвата"), theme::TEXT, "points", |ui| {
            for (i, p) in self.world.points.iter().enumerate() {
                if point_label(ui, i, p.owner).clicked() {
                    self.viewport.focus(p.pos);
                }
                // Что решил штаб каждой команды: сколько отрядов послал и зачем.
                for team in 0..2 {
                    let Some(plan) = self.world.plans[team].get(i) else { continue };
                    let why = [t("гарнизон"), t("атака"), t("захват"), t("подкрепления")][plan.priority.min(3) as usize];
                    let text = f("    {} отр. · {} · врагов рядом {}", &[&plan.squads, &why, &plan.enemies]);
                    ui.label(RichText::new(text).small().color(theme::TEAM[team].gamma_multiply(0.85)));
                }
            }
        });
    }

    fn remote_outliner(&mut self, ui: &mut egui::Ui) {
        let Some(w) = self.client.as_ref().and_then(|c| c.world.as_ref()) else {
            ui.label(RichText::new(t("Ждём ответа сервера…")).color(theme::TEXT_DIM));
            return;
        };
        for team in 0..2 {
            let mine = if w.welcome.team == team as u8 { t(" (мы)") } else { "" };
            ui.label(
                RichText::new(f("{}{} · убийств {}", &[&theme::team_name(team), &mine, &w.score[team]]))
                    .color(theme::TEAM[team]),
            );
        }
        let visible = w.soldiers.iter().flatten().count();
        ui.label(RichText::new(f("Сервер «{}»", &[&w.welcome.server_name])).color(theme::TEXT));
        ui.label(RichText::new(f("Видно солдат: {}", &[&visible])).color(theme::TEXT_DIM));
        egui::CollapsingHeader::new(t("Точки захвата")).default_open(true).show(ui, |ui| {
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
                        theme::panel(ui, Icon::Network, t("Подключение"), true, |ui| self.client_panel(ui));
                        theme::panel(ui, Icon::Keyboard, t("Управление"), true, controls_panel);
                    } else {
                        theme::panel(ui, Icon::Play, t("Симуляция"), true, |ui| self.simulation_panel(ui));
                        theme::panel(ui, Icon::Flag, t("Матч"), true, |ui| self.match_panel(ui));
                        theme::panel(ui, Icon::Server, t("Сеть"), true, |ui| self.network_panel(ui));
                        theme::panel(ui, Icon::Chart, t("Производительность"), true, |ui| self.performance_panel(ui));
                        theme::panel(ui, Icon::Cursor, t("Выделение"), true, |ui| self.selection_panel(ui));
                    }
                });
            });
    }

    fn simulation_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let (icon, label) = if self.paused { (Icon::Play, t("Пуск")) } else { (Icon::Pause, t("Пауза")) };
            if icons::button(ui, icon, label).clicked() {
                self.paused = !self.paused;
            }
            if icons::button_enabled(ui, self.paused, Icon::Step, t("Шаг")).clicked() {
                self.step();
            }
        });
        theme::props(ui, "sim_props", |ui| {
            theme::label(ui, t("Скорость"));
            egui::ComboBox::from_id_salt("speed")
                .selected_text(format!("×{}", self.speed))
                .show_ui(ui, |ui| {
                    for s in [0.25, 0.5, 1.0, 2.0, 4.0, 8.0, 16.0] {
                        ui.selectable_value(&mut self.speed, s, format!("×{s}"));
                    }
                });
            ui.end_row();
            theme::label(ui, t("Тиков в секунду"));
            ui.label(self.world.config.tick_rate.to_string());
            ui.end_row();
            theme::label(ui, t("ИИ думает"));
            ui.label(f("раз в {} тиков", &[&self.world.config.think_interval]));
            ui.end_row();
        });
        ui.label(
            RichText::new(t("Оружие, ИИ и гранаты правятся в data/sim.ron — изменения подхватываются на лету."))
                .small()
                .color(theme::TEXT_DIM),
        );
    }

    fn match_panel(&mut self, ui: &mut egui::Ui) {
        let from_file = self.map_file.as_ref().map(|f| f.name.clone());
        let round = &self.world.round;
        let names = &self.world.config.arsenal.team_names;
        let state = match round.phase {
            wind_sim::round::Phase::Playing => f("раунд {} · {:.0} с", &[&round.number, &round.time]),
            wind_sim::round::Phase::Over { winner: Some(t), .. } => f("раунд {} · победили {}", &[&round.number, &names[t as usize & 1]]),
            wind_sim::round::Phase::Over { winner: None, .. } => f("раунд {} · ничья", &[&round.number]),
        };
        theme::props(ui, "match_props", |ui| {
            theme::label(ui, t("Раунд"));
            ui.label(state);
            ui.end_row();
            theme::label(ui, t("Карта"));
            match &from_file {
                Some(name) => ui.label(f("«{}» из редактора", &[&name])),
                None => ui.label(t("процедурная")),
            };
            ui.end_row();
            if from_file.is_none() {
                theme::label(ui, t("Размер карты"));
                egui::ComboBox::from_id_salt("map_size")
                    .selected_text(f("{}×{} м", &[&self.settings.map_size, &self.settings.map_size]))
                    .show_ui(ui, |ui| {
                        for m in [256, 512, 1024, 2048] {
                            ui.selectable_value(&mut self.settings.map_size, m, f("{}×{} м", &[&m, &m]));
                        }
                    });
                ui.end_row();
                theme::label(ui, t("Точек захвата"));
                ui.add(egui::DragValue::new(&mut self.settings.capture_points).range(1..=26));
                ui.end_row();
            }
            theme::label(ui, t("Солдат в команде"));
            ui.add(egui::DragValue::new(&mut self.settings.soldiers_per_team).range(1..=10_000).speed(10));
            ui.end_row();
            theme::label(ui, "Seed");
            ui.add(egui::DragValue::new(&mut self.settings.seed));
            ui.end_row();
        });
        if from_file.is_some() && ui.button(t("Вернуться к процедурной карте")).clicked() {
            self.map_file = None;
            self.restart();
        }
        let changed = (from_file.is_none()
            && (self.settings.map_size != self.world.config.map_size
                || self.settings.capture_points != self.world.config.capture_points))
            || self.settings.soldiers_per_team != self.world.config.soldiers_per_team
            || self.settings.seed != self.world.seed;
        let text = if changed { t("Перезапустить с новыми параметрами") } else { t("Перезапустить матч") };
        if icons::wide_button(ui, Icon::Restart, text).clicked() {
            self.restart();
        }
    }

    fn network_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new(t("Свой сервер")).strong());
        match &self.server {
            None => {
                theme::props(ui, "server_props", |ui| {
                    theme::label(ui, t("Порт"));
                    ui.add(egui::DragValue::new(&mut self.server_port).range(1024..=65535));
                    ui.end_row();
                });
                if icons::wide_button(ui, Icon::Server, t("Запустить сервер")).clicked() {
                    self.start_server();
                }
            }
            Some(server) => {
                let down: f32 = server.clients.iter().map(|c| c.traffic.sent_per_sec).sum();
                let n = server.clients.len();
                theme::props(ui, "server_stats", |ui| {
                    theme::label(ui, t("Порт"));
                    ui.label(self.server_port.to_string());
                    ui.end_row();
                    theme::label(ui, t("Игроков"));
                    ui.label(format!("{n} / {}", server.max_clients));
                    ui.end_row();
                    theme::label(ui, t("На игрока"));
                    ui.label(f("{:.1} КБ/с", &[&(down / n.max(1) as f32 / 1024.0)]));
                    ui.end_row();
                    theme::label(ui, t("Отдаёт всего"));
                    ui.label(f("{:.0} КБ/с", &[&(server.traffic.sent_per_sec / 1024.0)]));
                    ui.end_row();
                });
                if icons::wide_button(ui, Icon::Pause, t("Остановить сервер")).clicked() {
                    // Без сервера игроки пропадут — отдаём их солдат обратно ИИ.
                    for c in &server.clients {
                        self.world.release_control(c.replica.soldier);
                    }
                    self.server = None;
                    self.log(t("Сервер остановлен").into());
                }
            }
        }
        ui.add_space(6.0);
        ui.label(RichText::new(t("Подключиться")).strong());
        theme::props(ui, "connect_props", |ui| {
            theme::label(ui, t("Адрес"));
            ui.add(egui::TextEdit::singleline(&mut self.connect_addr).desired_width(150.0));
            ui.end_row();
            theme::label(ui, t("Имя"));
            ui.add(egui::TextEdit::singleline(&mut self.player_name).desired_width(150.0));
            ui.end_row();
            theme::label(ui, t("Пароль"));
            ui.add(egui::TextEdit::singleline(&mut self.connect_password).password(true).desired_width(150.0));
            ui.end_row();
        });
        if icons::wide_button(ui, Icon::Network, t("Подключиться и играть")).clicked() {
            self.connect();
        }
        for line in &self.net_log {
            ui.label(RichText::new(line).small().color(theme::TEXT_DIM));
        }
    }

    fn client_panel(&mut self, ui: &mut egui::Ui) {
        let Some(client) = &mut self.client else { return };
        let status = match &client.status {
            ClientStatus::Connecting => t("подключаюсь…").to_string(),
            ClientStatus::Connected => t("в игре").to_string(),
            ClientStatus::Rejected(why) => f("отказ: {}", &[&why]),
            ClientStatus::TimedOut => t("сервер не отвечает").to_string(),
        };
        theme::props(ui, "client_props", |ui| {
            theme::label(ui, t("Сервер"));
            ui.label(client.server_addr().to_string());
            ui.end_row();
            theme::label(ui, t("Состояние"));
            ui.label(status);
            ui.end_row();
            theme::label(ui, t("Пинг"));
            ui.label(f("{:.0} мс", &[&client.rtt_ms]));
            ui.end_row();
            theme::label(ui, t("К нам"));
            ui.label(f("{:.1} КБ/с", &[&(client.traffic.recv_per_sec / 1024.0)]));
            ui.end_row();
            theme::label(ui, t("От нас"));
            ui.label(f("{:.2} КБ/с", &[&(client.traffic.sent_per_sec / 1024.0)]));
            ui.end_row();
            theme::label(ui, t("Пакет до"));
            ui.label(f("{} Б", &[&client.traffic.max_recv_packet]));
            ui.end_row();
            theme::label(ui, t("Не разобрано"));
            ui.label(client.decode_errors.to_string());
            ui.end_row();
            theme::label(ui, t("Терять пакеты"));
            ui.add(egui::Slider::new(&mut client.simulated_loss, 0.0..=0.5).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
            ui.end_row();
        });
        ui.label(
            RichText::new(t("«Терять пакеты» нарочно выбрасывает часть входящих снимков — проверить, как игра переживает плохую сеть."))
                .small()
                .color(theme::TEXT_DIM),
        );
        if icons::wide_button(ui, Icon::Pause, t("Отключиться")).clicked() {
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
            theme::label(ui, t("Тик, среднее"));
            ui.label(f("{:.2} мс из {:.0}", &[&total, &budget]));
            ui.end_row();
            theme::label(ui, t("Тик, худший"));
            ui.label(f("{:.2} мс", &[&worst]));
            ui.end_row();
            theme::label(ui, t("Пуль в полёте"));
            ui.label(self.world.stats.bullets.to_string());
            ui.end_row();
            theme::label(ui, t("Гранат"));
            ui.label(self.world.stats.grenades.to_string());
            ui.end_row();
        });
        ui.add_space(4.0);

        // Полоски по фазам: какая часть тика на что уходит.
        type Phase = (&'static str, fn(&TickStats) -> f32);
        let phases: [Phase; 7] = [
            (t("Сетка"), |s| s.grid_ms),
            (t("Пути"), |s| s.nav_ms),
            (t("ИИ"), |s| s.think_ms),
            (t("Движение"), |s| s.move_ms),
            (t("Стрельба"), |s| s.fire_ms),
            (t("Снаряды"), |s| s.projectiles_ms),
            (t("Точки"), |s| s.capture_ms),
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
            f("{:.1} мс", &[&scale]),
            egui::FontId::proportional(10.0),
            theme::TEXT_DIM,
        );
    }

    fn selection_panel(&mut self, ui: &mut egui::Ui) {
        let Some(i) = self.selected else {
            ui.label(RichText::new(t("Кликни по солдату во вьюпорте.")).color(theme::TEXT_DIM));
            return;
        };
        let s = &self.world.soldiers[i as usize];
        theme::props(ui, "sel_props", |ui| {
            theme::label(ui, t("Солдат"));
            ui.label(RichText::new(format!("#{i} · {}", theme::team_name(s.team as usize))).color(theme::TEAM[s.team as usize]));
            ui.end_row();
            theme::label(ui, t("Управляет"));
            ui.label(match s.controller {
                wind_sim::Controller::Ai => t("ИИ"),
                wind_sim::Controller::Player(_) => t("игрок по сети"),
            });
            ui.end_row();
            theme::label(ui, t("Состояние"));
            if s.alive {
                ui.label(f("жив, здоровье {:.0}%", &[&(s.health * 100.0)]));
            } else {
                ui.label(f("мёртв, появится через {:.1} с", &[&s.respawn_timer.max(0.0)]));
            }
            ui.end_row();
            theme::label(ui, t("Цель"));
            ui.label(s.target.map_or("—".into(), |t| f("солдат #{}", &[&t])));
            ui.end_row();
            theme::label(ui, t("Отряд"));
            match self.world.squads.get(s.squad as usize) {
                Some(q) => {
                    let role = match self.world.orders.get(i as usize) {
                        Some(wind_sim::squad::Order::Lead { .. }) => t("командир").to_string(),
                        Some(wind_sim::squad::Order::Follow { .. }) => q.leader.map_or(t("в строю").into(), |l| f("в строю, командир #{}", &[&l])),
                        _ => t("сам по себе").into(),
                    };
                    ui.label(format!("№{} · {role}", s.squad + 1));
                }
                None => {
                    ui.label("—");
                }
            }
            ui.end_row();
            theme::label(ui, t("Укрытие"));
            ui.label(match s.cover {
                Some(_) if s.reload_timer > 0.0 => t("прячется за угол (перезарядка)"),
                Some(_) => t("стреляет из-за угла"),
                None => "—",
            });
            ui.end_row();
            theme::label(ui, t("Под огнём"));
            ui.label(if s.under_fire > 0.0 { f("да, ещё {:.1} с", &[&s.under_fire]) } else { t("нет").into() });
            ui.end_row();
            theme::label(ui, t("Подавлен"));
            ui.label(format!(
                "{:.0}%{}",
                s.suppression * 100.0,
                if s.prone {
                    t(" · залёг")
                } else if s.pinned {
                    t(" · прижат, за углом")
                } else {
                    ""
                }
            ));
            ui.end_row();
            if s.retreat.is_some() {
                theme::label(ui, t("Ранен"));
                ui.label(t("отходит в тыл"));
                ui.end_row();
            }
            theme::label(ui, t("Идёт к"));
            ui.label(match s.objective {
                o if (o as usize) < self.world.points.len() => f("точке {}", &[&((b'A' + o as u8) as char)]),
                _ => "—".into(),
            });
            ui.end_row();
            theme::label(ui, t("Оружие"));
            ui.label(&self.world.weapon(i).name);
            ui.end_row();
            theme::label(ui, t("Патроны"));
            if s.reload_timer > 0.0 {
                ui.label(t("перезарядка…"));
            } else {
                ui.label(format!("{} / {}", s.ammo, self.world.weapon(i).magazine_size));
            }
            ui.end_row();
            theme::label(ui, t("Гранаты"));
            ui.label(s.grenades.to_string());
            ui.end_row();
            theme::label(ui, t("Убийств"));
            ui.label(s.kills.to_string());
            ui.end_row();
        });
        ui.checkbox(&mut self.viewport.follow, t("Следовать камерой"));
    }
}

fn controls_panel(ui: &mut egui::Ui) {
    theme::props(ui, "controls", |ui| {
        for (key, what) in [
            ("W A S D", t("идти")),
            (t("Мышь"), t("прицел")),
            (t("ЛКМ"), t("огонь")),
            ("G", t("граната")),
            (t("Колесо"), t("масштаб")),
            (t("СКМ / ПКМ"), t("двигать вид")),
        ] {
            theme::label(ui, key);
            ui.label(what);
            ui.end_row();
        }
    });
}

fn point_label(ui: &mut egui::Ui, index: usize, owner: Option<u8>) -> egui::Response {
    let (name, color) = match owner {
        Some(t) => (theme::team_name(t as usize), theme::TEAM[t as usize]),
        None => (t("ничья"), theme::TEXT_DIM),
    };
    let letter = (b'A' + index as u8) as char;
    icons::selectable(ui, false, Icon::Flag, &f("Точка {} — {}", &[&letter, &name]), Some(color))
}

impl Lab {
    /// Кадр лаборатории; возвращает просьбу к оболочке, если нажали кнопку в шапке.
    pub fn ui(&mut self, ui: &mut egui::Ui) -> Option<LabAction> {
        self.hot_reload();
        let typing = ui.ctx().egui_wants_keyboard_input();
        let (frame_dt, space, right) = ui.input(|i| {
            (i.stable_dt, i.key_pressed(egui::Key::Space), i.key_pressed(egui::Key::ArrowRight))
        });
        let editing = self.client.is_none() && self.workspace == Workspace::Map;
        if self.client.is_none() && !editing && !typing {
            if space {
                self.paused = !self.paused;
            }
            if right && self.paused {
                self.step();
            }
        }
        self.advance(frame_dt);

        let action = self.top_bar(ui);
        self.status_bar(ui);
        if editing {
            self.map_workspace(ui);
            ui.ctx().request_repaint();
            return action;
        }
        if self.client.is_none() && self.workspace == Workspace::Studio {
            self.studio_workspace(ui);
            ui.ctx().request_repaint();
            return action;
        }
        self.outliner(ui);
        self.properties(ui);
        let ctx = ui.ctx().clone();
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let playing = self.client.is_some();
            let scene = match &self.client {
                Some(client) => scene::remote(client),
                None => Some(scene::local(&self.world, self.selected, &self.blasts, self.accumulator)),
            };
            let Some(scene) = scene else {
                ui.centered_and_justified(|ui| ui.label(t("Ждём ответа сервера…")));
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
        action
    }
}

impl Lab {
    /// Рабочее пространство «Редактор карт».
    fn map_workspace(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        self.map_ws.shortcuts(&ctx);
        if self.map_ws.generation != self.map_generation {
            // Открыли или создали другую карту — подгоняем вид заново.
            self.map_generation = self.map_ws.generation;
            self.editor_view.reset();
        }
        let focus = egui::Panel::left("map_tools")
            .resizable(true)
            .default_size(200.0)
            .frame(egui::Frame::new().fill(theme::OUTLINER).inner_margin(egui::Margin::same(6)))
            .show(ui, |ui| self.map_ws.tool_shelf(ui))
            .inner;
        if let Some(pos) = focus {
            self.editor_view.focus(pos);
        }
        let request = egui::Panel::right("map_properties")
            .resizable(true)
            .default_size(320.0)
            .frame(egui::Frame::new().fill(theme::REGION).inner_margin(egui::Margin::same(6)))
            .show(ui, |ui| egui::ScrollArea::vertical().show(ui, |ui| self.map_ws.properties(ui)).inner)
            .inner;
        egui::CentralPanel::no_frame().show(ui, |ui| {
            let scene = self.map_ws.scene(&self.base_config);
            let out = self.editor_view.show(ui, &scene, true);
            drop(scene);
            self.map_ws.handle(&ctx, &out);
        });
        if let Some(editor_ui::Request::Play(file)) = request {
            self.log(f("Играем на карте «{}»", &[&file.name]));
            self.map_file = Some(file);
            self.restart();
            self.set_workspace(Workspace::Simulation);
        }
    }

    /// Рабочее пространство «Студия персонажей».
    fn studio_workspace(&mut self, ui: &mut egui::Ui) {
        egui::Panel::left("studio_models")
            .resizable(true)
            .default_size(200.0)
            .frame(egui::Frame::new().fill(theme::OUTLINER).inner_margin(egui::Margin::same(6)))
            .show(ui, |ui| egui::ScrollArea::vertical().show(ui, |ui| self.studio.outliner(ui)));
        egui::Panel::right("studio_properties")
            .resizable(true)
            .default_size(320.0)
            .frame(egui::Frame::new().fill(theme::REGION).inner_margin(egui::Margin::same(6)))
            .show(ui, |ui| egui::ScrollArea::vertical().show(ui, |ui| self.studio.properties(ui)));
        egui::CentralPanel::no_frame().show(ui, |ui| self.studio.viewport(ui));
        // Сохранили клип — модели перечитаны: и в остальных видах новые движения.
        if let Some(set) = self.studio.take_reloaded() {
            for v in [&mut self.viewport, &mut self.editor_view] {
                v.set_characters(Some(set.clone()));
            }
        }
    }

    /// Открыть карту проекта в редакторе карт (с главного экрана).
    pub fn open_map(&mut self, name: &str) {
        self.map_ws.open(name);
        self.set_workspace(Workspace::Map);
    }

    pub fn set_workspace(&mut self, workspace: Workspace) {
        if self.workspace != workspace {
            self.workspace = workspace;
            // 3D-отрисовщик помнит одну карту — показываемый вьюпорт должен её прислать заново.
            match workspace {
                Workspace::Simulation => self.viewport.refresh_map(),
                Workspace::Map => self.editor_view.refresh_map(),
                Workspace::Studio => self.studio.view.refresh_map(),
            }
        }
    }
}

fn with_settings(base: &SimConfig, s: &MatchSettings) -> SimConfig {
    let mut config = base.clone();
    config.map_size = s.map_size;
    config.soldiers_per_team = s.soldiers_per_team;
    config.capture_points = s.capture_points;
    config
}

fn mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}
