//! Лаборатория: смотрим и крутим симуляцию в окне.
//!
//! cargo run --release -p wind_lab

mod theme;
mod viewport;

use eframe::egui::{self, Color32, RichText};
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};
use viewport::{Blast, Viewport};
use wind_sim::{Event, SimConfig, TickStats, World};

/// Сколько последних тиков держим для графика.
const HISTORY: usize = 300;
/// Больше тиков за кадр не считаем, даже если не успеваем, — иначе окно зависнет.
const MAX_STEPS_PER_FRAME: u32 = 64;

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("The Wind Engine — Лаборатория")
            .with_inner_size([1600.0, 950.0]),
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
    status: (String, Color32),
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
            viewport: Viewport::new(),
            status,
        }
    }

    fn restart(&mut self) {
        self.world = World::new(with_settings(&self.base_config, &self.settings), self.settings.seed);
        self.history.clear();
        self.selected = None;
        self.accumulator = 0.0;
        self.viewport.reset();
    }

    fn step(&mut self) {
        self.world.step();
        for event in &self.world.events {
            if let Event::Explosion { pos, radius } = *event {
                self.viewport.blasts.push(Blast { pos, radius, age: 0.0 });
            }
        }
        if self.history.len() == HISTORY {
            self.history.pop_front();
        }
        self.history.push_back(self.world.stats.clone());
    }

    fn advance(&mut self, frame_dt: f32) {
        for b in &mut self.viewport.blasts {
            b.age += frame_dt;
        }
        self.viewport.blasts.retain(|b| b.age < 0.6);
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
                        let o = &mut self.viewport.overlays;
                        ui.checkbox(&mut o.grid, "Сетка");
                        ui.checkbox(&mut o.bullets, "Пули и гранаты");
                        ui.checkbox(&mut o.vision, "Обзор выделенного");
                        ui.checkbox(&mut o.flow, "Поле путей выделенного");
                        ui.checkbox(&mut self.viewport.follow, "Следовать за выделенным");
                    });
                    ui.add_space(24.0);
                    // Вкладки рабочих пространств, как Layout / Modeling в Blender.
                    let _ = ui.selectable_label(true, "Лаборатория");
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
                        let s = &self.world.stats;
                        let fps = 1.0 / ui.input(|i| i.stable_dt).max(1e-4);
                        ui.label(RichText::new(format!(
                            "Тик {} · {:.1} с · {:.2} мс/тик · {:.0} FPS",
                            self.world.tick,
                            self.world.tick as f32 * self.world.dt(),
                            s.total_ms,
                            fps
                        ))
                        .small()
                        .color(theme::TEXT_DIM));
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
                    egui::CollapsingHeader::new("Точки захвата").default_open(true).show(ui, |ui| {
                        for (i, p) in self.world.points.iter().enumerate() {
                            let (owner, color) = match p.owner {
                                Some(t) => (theme::TEAM_NAME[t as usize], theme::TEAM[t as usize]),
                                None => ("ничья", theme::TEXT_DIM),
                            };
                            let letter = (b'A' + i as u8) as char;
                            if ui.selectable_label(false, RichText::new(format!("Точка {letter} — {owner}")).color(color)).clicked() {
                                self.viewport.focus(p.pos);
                            }
                        }
                    });
                });
            });
    }

    fn properties(&mut self, ui: &mut egui::Ui) {
        egui::Panel::right("properties")
            .resizable(true)
            .default_size(300.0)
            .frame(egui::Frame::new().fill(theme::REGION).inner_margin(egui::Margin::same(6)))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    theme::panel(ui, "Симуляция", true, |ui| self.simulation_panel(ui));
                    theme::panel(ui, "Матч", true, |ui| self.match_panel(ui));
                    theme::panel(ui, "Производительность", true, |ui| self.performance_panel(ui));
                    theme::panel(ui, "Выделение", true, |ui| self.selection_panel(ui));
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

impl eframe::App for Lab {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.hot_reload();
        let (frame_dt, space, right) = ui.input(|i| {
            (i.stable_dt, i.key_pressed(egui::Key::Space), i.key_pressed(egui::Key::ArrowRight))
        });
        if space {
            self.paused = !self.paused;
        }
        if right && self.paused {
            self.step();
        }
        self.advance(frame_dt);

        self.top_bar(ui);
        self.status_bar(ui);
        self.outliner(ui);
        self.properties(ui);
        egui::CentralPanel::no_frame().show(ui, |ui| {
            if let Some(pick) = self.viewport.show(ui, &self.world, self.selected) {
                self.selected = pick;
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
