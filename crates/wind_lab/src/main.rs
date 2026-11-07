//! The Wind Engine — движок: окно выбора проекта, главный экран проекта и редакторы
//! (лаборатория, редактор карт, студия персонажей).
//!
//! cargo run --release -p wind_lab                 — окно выбора проекта
//! cargo run --release -p wind_lab -- --project <папка>   — сразу главный экран проекта
//! cargo run --release -p wind_lab -- --editor     — сразу в редактор (и другие ключи
//!     лаборатории: --studio, --connect, --serve, --3d, --open, --play-map, --follow);
//!     без --project — последний открытый проект или наша игра
//! cargo run --release -p wind_lab -- --lang en  — язык на этот запуск (en / ru)
//! cargo run --release -p wind_lab -- --settings  — сразу открыть настройки движка
//! cargo run --release -p wind_lab -- --import-mocap запись.bvh run "источник"
//!     — круг шага из записи движений в data/animations/run.ron (с кадрами
//!       `from to` после источника — действие целиком, не круг)

mod anim_editor;
mod editor;
mod editor_ui;
mod hub;
mod i18n;
mod icons;
mod lab;
mod launcher;
mod project;
mod scene;
mod settings;
mod studio;
mod theme;
mod viewport;

use eframe::egui::{self, RichText};
use hub::{Hub, HubAction};
use i18n::{Lang, f, t};
use icons::Icon;
use lab::{Lab, LabAction, MSAA};
use launcher::{Launcher, LauncherAction};
use project::Project;
use settings::EngineSettings;
use std::path::{Path, PathBuf};

/// Размер окна с проектом, точки.
const WORK_SIZE: [f32; 2] = [1600.0, 950.0];

fn main() -> eframe::Result {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // --lang en / --lang ru — язык на этот запуск, настройки не меняются (для проверки перевода).
    let lang = args.iter().position(|a| a == "--lang").map(|i| {
        let value = args.get(i + 1).cloned().unwrap_or_default();
        args.drain(i..(i + 2).min(args.len()));
        if value.eq_ignore_ascii_case("en") { Lang::En } else { Lang::Ru }
    });
    if args.first().is_some_and(|a| a == "--import-mocap") {
        let data = project::sample().map_or_else(|| PathBuf::from("data"), |p| p.data_dir());
        match import_mocap(&args[1..], &data) {
            Ok(text) => println!("{text}"),
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
        return Ok(());
    }
    // --settings — сразу открыть окно настроек (для проверки и снимков).
    let settings_open = args.iter().any(|a| a == "--settings");
    args.retain(|a| a != "--settings");
    let start = Start::from_args(&args);
    let launcher = matches!(start, Start::Launcher);
    let viewport = egui::ViewportBuilder::default()
        .with_title("The Wind Engine")
        .with_icon(wind_render::app_icon::initial())
        // Окно проектов — небольшое и постоянного размера, как у Blender и Workbench.
        .with_inner_size(if launcher { launcher::SIZE } else { WORK_SIZE })
        .with_resizable(!launcher)
        .with_maximize_button(!launcher);
    let options = eframe::NativeOptions {
        viewport,
        centered: true,
        // Для 3D-вида: буфер глубины и сглаживание, как в игре.
        depth_buffer: 32,
        multisampling: MSAA as u16,
        ..Default::default()
    };
    eframe::run_native("wind_lab", options, Box::new(move |cc| Ok(Box::new(Engine::new(cc, start, lang, settings_open)))))
}

/// С чего начать: с окна проектов или сразу с проекта.
enum Start {
    Launcher,
    /// Проект (или последний / наша игра) и ключи лаборатории; пустые ключи — главный экран.
    Project { root: Option<PathBuf>, lab_args: Vec<String> },
}

impl Start {
    fn from_args(args: &[String]) -> Start {
        let mut root = None;
        let mut lab_args = Vec::new();
        let mut it = args.iter();
        while let Some(a) = it.next() {
            if a == "--project" {
                root = it.next().map(PathBuf::from);
            } else {
                lab_args.push(a.clone());
            }
        }
        if root.is_none() && lab_args.is_empty() { Start::Launcher } else { Start::Project { root, lab_args } }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Launcher,
    Hub,
    Editor,
}

/// Оболочка движка: кто сейчас на экране и открытый проект.
struct Engine {
    settings: EngineSettings,
    settings_path: PathBuf,
    settings_open: bool,
    /// Наша игра — шаблон новых проектов.
    sample: Option<Project>,
    render_state: Option<eframe::egui_wgpu::RenderState>,
    stage: Stage,
    launcher: Launcher,
    hub: Hub,
    project: Option<Project>,
    lab: Option<Lab>,
    icon: wind_render::app_icon::Follower,
}

impl Engine {
    fn new(cc: &eframe::CreationContext<'_>, start: Start, lang: Option<Lang>, settings_open: bool) -> Self {
        theme::apply(&cc.egui_ctx);
        let settings_path = EngineSettings::path();
        let settings = EngineSettings::load(&settings_path);
        i18n::set(lang.unwrap_or(settings.language));
        cc.egui_ctx.set_zoom_factor(settings.ui_scale);
        let sample = project::sample();
        let mut engine = Engine {
            launcher: Launcher::new(&settings, sample.as_ref()),
            settings,
            settings_path,
            settings_open,
            sample,
            render_state: cc.wgpu_render_state.clone(),
            stage: Stage::Launcher,
            hub: Hub::new(),
            project: None,
            lab: None,
            icon: Default::default(),
        };
        if let Start::Project { root, lab_args } = start {
            let root = root
                .or_else(|| engine.settings.recent.iter().find(|p| p.join(project::FILE).exists()).cloned())
                .or_else(|| engine.sample.as_ref().map(|p| p.root.clone()));
            if let Some(root) = root {
                engine.open_project(&cc.egui_ctx, &root, false);
                if let Some(lab) = &mut engine.lab
                    && !lab_args.is_empty()
                {
                    lab.apply_args(&lab_args);
                    engine.stage = Stage::Editor;
                }
            }
        }
        engine
    }

    /// Открыть проект: лаборатория для него и главный экран. `resize` — окно было окном проектов.
    fn open_project(&mut self, ctx: &egui::Context, root: &Path, resize: bool) {
        let project = match Project::open(root) {
            Ok(p) => p,
            Err(e) => {
                self.launcher.show_error(f("Не открылся проект: {}", &[&e]));
                return;
            }
        };
        self.settings.touch(&project.root);
        self.save_settings();
        self.lab = Some(Lab::new(self.render_state.as_ref(), &project));
        self.hub.refresh(&project);
        self.hub.message = None;
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(window_title(&project)));
        self.project = Some(project);
        self.stage = Stage::Hub;
        if resize {
            set_window(ctx, WORK_SIZE, true);
        }
    }

    fn close_project(&mut self, ctx: &egui::Context) {
        self.lab = None;
        self.project = None;
        self.stage = Stage::Launcher;
        self.launcher.refresh(&self.settings, self.sample.as_ref());
        ctx.send_viewport_cmd(egui::ViewportCommand::Title("The Wind Engine".into()));
        set_window(ctx, launcher::SIZE, false);
    }

    fn save_settings(&mut self) {
        if let Err(e) = self.settings.save(&self.settings_path) {
            eprintln!("настройки движка не сохранились: {e}");
        }
    }

    fn play(&mut self) {
        let Some(project) = &self.project else { return };
        // Игра лежит рядом с движком; данные берёт из текущей папки — папки проекта.
        let exe = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join(if cfg!(windows) { "wind_client.exe" } else { "wind_client" })));
        let result = match exe {
            Some(exe) if exe.exists() => std::process::Command::new(&exe).arg("--window").current_dir(&project.root).spawn().map(|_| ()).map_err(|e| e.to_string()),
            _ => Err(t("рядом с движком нет wind_client — собери игру: cargo build --release -p wind_client").to_owned()),
        };
        self.hub.message = Some(match result {
            Ok(()) => (t("Игра запускается в отдельном окне…").to_owned(), theme::TEXT_DIM),
            Err(e) => (f("Игра не запустилась: {}", &[&e]), theme::ERROR),
        });
    }

    /// Окно настроек движка: язык и масштаб интерфейса.
    fn settings_window(&mut self, ctx: &egui::Context) {
        let mut open = self.settings_open;
        let mut changed = false;
        let modal = egui::Modal::new(egui::Id::unique("engine_settings")).show(ctx, |ui| {
            ui.set_width(380.0);
            ui.horizontal(|ui| {
                icons::label(ui, Icon::Gear, t("Настройки движка"), theme::TEXT);
            });
            ui.separator();
            ui.add_space(6.0);
            egui::Grid::new("settings_grid").num_columns(2).spacing([16.0, 10.0]).show(ui, |ui| {
                icons::label(ui, Icon::Globe, t("Язык"), theme::TEXT_DIM);
                ui.horizontal(|ui| {
                    for lang in Lang::ALL {
                        if ui.selectable_label(self.settings.language == lang, lang.native_name()).clicked() && self.settings.language != lang {
                            self.settings.language = lang;
                            i18n::set(lang);
                            changed = true;
                        }
                    }
                });
                ui.end_row();
                ui.label(RichText::new(t("Масштаб интерфейса")).color(theme::TEXT_DIM));
                egui::ComboBox::from_id_salt("ui_scale").selected_text(format!("{:.0} %", self.settings.ui_scale * 100.0)).show_ui(ui, |ui| {
                    for scale in [0.8, 0.9, 1.0, 1.1, 1.25, 1.5] {
                        if ui.selectable_value(&mut self.settings.ui_scale, scale, format!("{:.0} %", scale * 100.0)).changed() {
                            changed = true;
                        }
                    }
                });
                ui.end_row();
            });
            ui.add_space(6.0);
            ui.label(RichText::new(t("Настройки общие для всех проектов и сохраняются сразу.")).small().color(theme::TEXT_DIM));
            ui.add_space(10.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::primary_button(ui, t("Готово")).clicked() {
                    open = false;
                }
            });
        });
        if modal.should_close() {
            open = false;
        }
        if changed {
            ctx.set_zoom_factor(self.settings.ui_scale);
            self.save_settings();
        }
        self.settings_open = open;
    }
}

impl eframe::App for Engine {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.icon.update(&ctx);
        match self.stage {
            Stage::Launcher => match self.launcher.ui(ui, &mut self.settings, self.sample.as_ref()) {
                Some(LauncherAction::Open(root) | LauncherAction::Created(root)) => self.open_project(&ctx, &root, true),
                Some(LauncherAction::Settings) => self.settings_open = true,
                Some(LauncherAction::SaveSettings) => self.save_settings(),
                None => {}
            },
            Stage::Hub => {
                let (Some(project), Some(lab)) = (&self.project, &mut self.lab) else {
                    self.stage = Stage::Launcher;
                    return;
                };
                match self.hub.ui(ui, project) {
                    Some(HubAction::Open(ws)) => {
                        lab.set_workspace(ws);
                        self.stage = Stage::Editor;
                    }
                    Some(HubAction::OpenMap(name)) => {
                        lab.open_map(&name);
                        self.stage = Stage::Editor;
                    }
                    Some(HubAction::Play) => self.play(),
                    Some(HubAction::ShowFolder) => show_folder(&project.root),
                    Some(HubAction::ChangeProject) => self.close_project(&ctx),
                    Some(HubAction::Settings) => self.settings_open = true,
                    None => {}
                }
            }
            Stage::Editor => {
                let Some(lab) = &mut self.lab else {
                    self.stage = Stage::Launcher;
                    return;
                };
                match lab.ui(ui) {
                    Some(LabAction::Home) => {
                        if let Some(p) = &self.project {
                            self.hub.refresh(p);
                        }
                        self.stage = Stage::Hub;
                    }
                    Some(LabAction::ChangeProject) => self.close_project(&ctx),
                    Some(LabAction::Settings) => self.settings_open = true,
                    None => {}
                }
            }
        }
        if self.settings_open {
            self.settings_window(&ctx);
        }
    }
}

/// Заголовок окна: «Проект — The Wind Engine»; наша игра тоже зовётся The Wind Engine — без повтора.
fn window_title(project: &Project) -> String {
    if project.name() == "The Wind Engine" { project.name().to_owned() } else { format!("{} — The Wind Engine", project.name()) }
}

/// Поменять размер окна и поставить его в центр экрана.
fn set_window(ctx: &egui::Context, size: [f32; 2], resizable: bool) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Resizable(resizable));
    ctx.send_viewport_cmd(egui::ViewportCommand::EnableButtons { close: true, minimized: true, maximize: resizable });
    ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(size.into()));
    if let Some(monitor) = ctx.input(|i| i.viewport().monitor_size) {
        let corner = ((monitor - egui::Vec2::from(size)) * 0.5).max(egui::Vec2::ZERO);
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(corner.to_pos2()));
    }
}

/// Папка в проводнике.
fn show_folder(path: &Path) {
    let program = if cfg!(windows) { "explorer" } else { "xdg-open" };
    let _ = std::process::Command::new(program).arg(path).spawn();
}

/// Запись движений (BVH) — в клип `data/animations/<имя>.ron` (см. `wind_render::mocap`).
/// Аргументы: файл, имя клипа, источник и, для действия, кадры `from to`.
fn import_mocap(args: &[String], data: &Path) -> Result<String, String> {
    let [file, name, source, rest @ ..] = args else {
        return Err("нужно: --import-mocap файл.bvh имя \"источник\" [from to]".into());
    };
    let text = std::fs::read_to_string(file).map_err(|e| format!("{file}: {e}"))?;
    let bvh = wind_render::mocap::Bvh::parse(&text)?;
    let clip = match rest {
        [from, to] => {
            let f = |v: &String| v.parse::<usize>().map_err(|_| format!("не номер кадра: {v}"));
            wind_render::mocap::Clip::action_from(&bvh, f(from)?, f(to)?, source)?
        }
        [] => wind_render::mocap::Clip::cycle_from(&bvh, source)?,
        _ => return Err("кадры — два числа: from to".into()),
    };
    let dir = data.join("animations");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(format!("{name}.ron"));
    let body = clip.to_ron()?;
    let header = format!("// Клип «{name}» из записи движений. Источник: {source}.\n// Сделан командой wind_lab --import-mocap; оси наши: x — вперёд, y — вверх, z — вправо.\n");
    std::fs::write(&path, header + &body).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(format!("{}: {} кадров, {:.2} с записи, путь за круг {:.2} длины ноги, пик на {:.0} %", path.display(), clip.frames.len(), clip.duration, clip.stride, clip.peak * 100.0))
}
