//! Окно выбора проекта — первое, что видно при запуске движка: недавние проекты,
//! наша игра, создание нового проекта и открытие папки.

use crate::i18n::{f, t};
use crate::icons::{self, Icon};
use crate::project::{self, CreateError, Project, Template};
use crate::settings::{EngineSettings, normalize};
use crate::theme;
use eframe::egui::{self, Color32, FontId, Margin, RichText, Stroke};
use std::path::PathBuf;

/// Размер окна выбора проекта, точки.
pub const SIZE: [f32; 2] = [1060.0, 660.0];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Page {
    Projects,
    New,
    Open,
}

/// Строка списка проектов.
struct Entry {
    root: PathBuf,
    /// `None` — папки нет или в ней нет `wind.project`.
    project: Option<Project>,
    /// Наша игра — всегда первая и не убирается из списка.
    sample: bool,
}

pub enum LauncherAction {
    Open(PathBuf),
    /// Новый проект создан — открыть его.
    Created(PathBuf),
    Settings,
    /// Список недавних изменился — сохранить настройки.
    SaveSettings,
}

pub struct Launcher {
    page: Page,
    entries: Vec<Entry>,
    new_name: String,
    new_parent: String,
    template: Template,
    open_path: String,
    /// Ошибка последнего действия — показывается внизу страницы.
    error: Option<String>,
}

impl Launcher {
    pub fn new(settings: &EngineSettings, sample: Option<&Project>) -> Self {
        let mut l = Launcher {
            page: Page::Projects,
            entries: Vec::new(),
            new_name: String::new(),
            new_parent: project::default_parent().display().to_string(),
            template: Template::Game,
            open_path: String::new(),
            error: None,
        };
        l.refresh(settings, sample);
        l
    }

    /// Перечитать список: наша игра и недавние проекты.
    pub fn refresh(&mut self, settings: &EngineSettings, sample: Option<&Project>) {
        self.entries.clear();
        if let Some(s) = sample {
            self.entries.push(Entry { root: s.root.clone(), project: Some(s.clone()), sample: true });
        }
        for root in &settings.recent {
            if sample.is_some_and(|s| normalize(root) == s.root) {
                continue;
            }
            self.entries.push(Entry { root: root.clone(), project: Project::open(root).ok(), sample: false });
        }
    }

    /// Не открылся проект — показать почему.
    pub fn show_error(&mut self, text: String) {
        self.error = Some(text);
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, settings: &mut EngineSettings, sample: Option<&Project>) -> Option<LauncherAction> {
        let mut action = None;
        egui::Panel::left("launcher_side")
            .exact_size(250.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(theme::TOPBAR).inner_margin(Margin { left: 18, right: 18, top: 26, bottom: 18 }))
            .show(ui, |ui| self.side(ui, &mut action));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BACKDROP).inner_margin(Margin { left: 28, right: 28, top: 24, bottom: 20 }))
            .show(ui, |ui| match self.page {
                Page::Projects => self.projects(ui, settings, sample, &mut action),
                Page::New => self.new_project(ui, sample, &mut action),
                Page::Open => self.open_folder(ui, &mut action),
            });
        action
    }

    fn side(&mut self, ui: &mut egui::Ui, action: &mut Option<LauncherAction>) {
        ui.horizontal(|ui| {
            theme::logo_image(ui, 46.0);
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.add_space(3.0);
                ui.label(RichText::new("The Wind Engine").font(FontId::proportional(19.0)).strong());
                ui.label(RichText::new(f("версия {}", &[&env!("CARGO_PKG_VERSION")])).small().color(theme::TEXT_DIM));
            });
        });
        ui.add_space(28.0);
        let pages = [(Page::Projects, Icon::Folder, t("Проекты")), (Page::New, Icon::New, t("Новый проект")), (Page::Open, Icon::Open, t("Открыть папку…"))];
        for (page, icon, text) in pages {
            if theme::side_button(ui, icon, text, self.page == page).clicked() {
                self.page = page;
                self.error = None;
            }
            ui.add_space(2.0);
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            if theme::side_button(ui, Icon::Gear, t("Настройки"), false).clicked() {
                *action = Some(LauncherAction::Settings);
            }
        });
    }

    fn projects(&mut self, ui: &mut egui::Ui, settings: &mut EngineSettings, sample: Option<&Project>, action: &mut Option<LauncherAction>) {
        heading(ui, t("Проекты"), t("Выбери проект, чтобы открыть его в движке."));
        let mut forget = None;
        egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| {
            for entry in &self.entries {
                match project_card(ui, entry) {
                    Some(CardClick::Open) => *action = Some(LauncherAction::Open(entry.root.clone())),
                    Some(CardClick::Forget) => forget = Some(entry.root.clone()),
                    None => {}
                }
                ui.add_space(8.0);
            }
            if self.entries.is_empty() {
                ui.label(RichText::new(t("Пока нет проектов — создай новый или открой папку.")).color(theme::TEXT_DIM));
            }
            if let Some(e) = &self.error {
                ui.add_space(6.0);
                ui.label(RichText::new(e).color(theme::ERROR));
            }
        });
        if let Some(root) = forget {
            settings.forget(&root);
            self.refresh(settings, sample);
            *action = Some(LauncherAction::SaveSettings);
        }
    }

    fn new_project(&mut self, ui: &mut egui::Ui, sample: Option<&Project>, action: &mut Option<LauncherAction>) {
        heading(ui, t("Новый проект"), t("Проект — папка с файлом wind.project и данными игры (data)."));
        ui.label(RichText::new(t("Шаблон")).strong());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let templates = [
                (Template::Game, Icon::Simulation, t("Копия нашей игры"), t("Карты, солдаты, оружие, дома и трава из «The Wind Engine» — начать с готового.")),
                (Template::Empty, Icon::New, t("Пустой"), t("Только параметры боя (sim.ron): процедурная карта и встроенные модели.")),
            ];
            for (template, icon, title, about) in templates {
                if template_card(ui, icon, title, about, self.template == template).clicked() {
                    self.template = template;
                }
                ui.add_space(8.0);
            }
        });
        ui.add_space(16.0);
        egui::Grid::new("new_project").num_columns(2).spacing([12.0, 10.0]).show(ui, |ui| {
            ui.label(t("Название"));
            ui.add(egui::TextEdit::singleline(&mut self.new_name).hint_text(t("Моя игра")).desired_width(420.0));
            ui.end_row();
            ui.label(t("Где создать"));
            ui.add(egui::TextEdit::singleline(&mut self.new_parent).desired_width(420.0));
            ui.end_row();
        });
        let name = self.new_name.trim().to_owned();
        if !name.is_empty() {
            let target = PathBuf::from(self.new_parent.trim()).join(&name);
            ui.label(RichText::new(f("Будет создана папка {}", &[&target.display()])).small().color(theme::TEXT_DIM));
        }
        ui.add_space(14.0);
        ui.horizontal(|ui| {
            let ready = sample.is_some() && !name.is_empty();
            if ui.add_enabled_ui(ready, |ui| theme::primary_button(ui, t("Создать"))).inner.clicked() {
                let parent = PathBuf::from(self.new_parent.trim());
                match Project::create(&parent, &self.new_name, self.template, sample.expect("ready")) {
                    Ok(p) => {
                        self.new_name.clear();
                        self.page = Page::Projects;
                        self.error = None;
                        *action = Some(LauncherAction::Created(p.root));
                    }
                    Err(e) => self.error = Some(create_error(&e)),
                }
            }
            if ui.add_sized([96.0, 28.0], egui::Button::new(t("Отмена"))).clicked() {
                self.page = Page::Projects;
                self.error = None;
            }
        });
        if sample.is_none() {
            ui.label(RichText::new(t("Не найдена наша игра (wind.project) — шаблоны взять неоткуда.")).color(theme::ERROR));
        }
        if let Some(e) = &self.error {
            ui.add_space(6.0);
            ui.label(RichText::new(e).color(theme::ERROR));
        }
    }

    fn open_folder(&mut self, ui: &mut egui::Ui, action: &mut Option<LauncherAction>) {
        heading(ui, t("Открыть папку"), t("Укажи папку проекта — ту, где лежит файл wind.project."));
        let hint = project::default_parent().join(t("Моя игра")).display().to_string();
        let edit = ui.add(egui::TextEdit::singleline(&mut self.open_path).hint_text(hint).desired_width(560.0));
        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.add_space(12.0);
        let path = self.open_path.trim().trim_matches('"');
        if (theme::primary_button(ui, t("Открыть")).clicked() || enter) && !path.is_empty() {
            let root = PathBuf::from(path);
            // Указали сам файл wind.project — берём его папку.
            let root = if root.file_name().is_some_and(|n| n == project::FILE) { root.parent().map(PathBuf::from).unwrap_or(root) } else { root };
            match Project::open(&root) {
                Ok(p) => {
                    self.error = None;
                    *action = Some(LauncherAction::Open(p.root));
                }
                Err(_) => self.error = Some(f("В папке {} нет файла wind.project", &[&root.display()])),
            }
        }
        if let Some(e) = &self.error {
            ui.add_space(8.0);
            ui.label(RichText::new(e).color(theme::ERROR));
        }
    }
}

fn create_error(e: &CreateError) -> String {
    match e {
        CreateError::NoName => t("Нужно название проекта.").into(),
        CreateError::BadName => t("В названии нельзя знаки < > : \" / \\ | ? * и точку в конце.").into(),
        CreateError::NotEmpty(p) => f("Папка {} уже есть и не пуста.", &[&p.display()]),
        CreateError::Io(e) => f("Не получилось создать проект: {}", &[e]),
    }
}

/// Заголовок страницы и строка пояснения под ним.
fn heading(ui: &mut egui::Ui, title: &str, about: &str) {
    ui.label(RichText::new(title).font(FontId::proportional(22.0)).strong());
    ui.add_space(2.0);
    ui.label(RichText::new(about).color(theme::TEXT_DIM));
    ui.add_space(16.0);
}

enum CardClick {
    Open,
    Forget,
}

/// Карточка проекта: значок с буквой, название, путь, описание и кнопки.
fn project_card(ui: &mut egui::Ui, entry: &Entry) -> Option<CardClick> {
    let mut click = None;
    let width = ui.available_width();
    let frame = egui::Frame::new().fill(theme::SUBPANEL).corner_radius(6).inner_margin(Margin::same(12));
    let r = frame.show(ui, |ui| {
        ui.set_width(width - 24.0);
        ui.horizontal(|ui| {
            let name = entry.project.as_ref().map_or_else(|| entry.root.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned()), |p| p.name().to_owned());
            badge(ui, &name, entry.project.is_some());
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&name).font(FontId::proportional(16.0)).strong());
                    if entry.sample {
                        tag(ui, t("наша игра"));
                    }
                });
                ui.label(RichText::new(entry.root.display().to_string()).small().color(theme::TEXT_DIM));
                match &entry.project {
                    Some(p) if !p.manifest.description.is_empty() => {
                        ui.add(egui::Label::new(RichText::new(&p.manifest.description).small()).truncate());
                    }
                    Some(_) => {}
                    None => {
                        ui.label(RichText::new(t("Папка не найдена или в ней нет wind.project")).small().color(theme::ERROR));
                    }
                }
            });
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if entry.project.is_some() {
                    if theme::primary_button(ui, t("Открыть")).clicked() {
                        click = Some(CardClick::Open);
                    }
                } else if ui.button(t("Убрать из списка")).clicked() {
                    click = Some(CardClick::Forget);
                }
            });
        });
    });
    // Двойной щелчок по карточке — тоже открыть.
    let card = ui.interact(r.response.rect, ui.scope_id().with(("card", &entry.root)), egui::Sense::click());
    if card.hovered() {
        let rect = r.response.rect;
        ui.painter().rect_stroke(rect, 6.0, Stroke::new(1.0, theme::ACCENT_BRIGHT), egui::StrokeKind::Inside);
    }
    if card.double_clicked() && entry.project.is_some() {
        click = Some(CardClick::Open);
    }
    if !entry.sample && entry.project.is_some() {
        card.context_menu(|ui| {
            if ui.button(t("Убрать из списка")).clicked() {
                click = Some(CardClick::Forget);
            }
        });
    }
    click
}

/// Квадрат с первой буквой названия — «обложка» проекта.
fn badge(ui: &mut egui::Ui, name: &str, ok: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(44.0, 44.0), egui::Sense::hover());
    let fill = if ok { theme::ACCENT } else { theme::WIDGET };
    ui.painter().rect_filled(rect, 6.0, fill);
    let letter: String = name.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
    ui.painter().text(rect.center(), egui::Align2::CENTER_CENTER, letter, FontId::proportional(22.0), Color32::WHITE);
}

/// Маленькая метка рядом с названием.
fn tag(ui: &mut egui::Ui, text: &str) {
    egui::Frame::new()
        .fill(theme::ACCENT.gamma_multiply(0.6))
        .corner_radius(3)
        .inner_margin(Margin { left: 6, right: 6, top: 1, bottom: 1 })
        .show(ui, |ui| ui.label(RichText::new(text).small().color(theme::TEXT)));
}

/// Карточка шаблона нового проекта.
fn template_card(ui: &mut egui::Ui, icon: Icon, title: &str, about: &str, selected: bool) -> egui::Response {
    let size = egui::vec2(300.0, 92.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let p = ui.painter();
    p.rect_filled(rect, 6.0, if response.hovered() { theme::WIDGET } else { theme::SUBPANEL });
    if selected {
        p.rect_stroke(rect, 6.0, Stroke::new(2.0, theme::ACCENT_BRIGHT), egui::StrokeKind::Inside);
    }
    let at = egui::Rect::from_min_size(rect.min + egui::vec2(14.0, 14.0), egui::vec2(20.0, 20.0));
    icons::paint(p, at, icon, if selected { theme::ACCENT_BRIGHT } else { theme::TEXT });
    p.text(rect.min + egui::vec2(44.0, 14.0), egui::Align2::LEFT_TOP, title, FontId::proportional(15.0), theme::TEXT);
    let galley = p.layout(about.to_owned(), FontId::proportional(12.0), theme::TEXT_DIM, size.x - 58.0);
    p.galley(rect.min + egui::vec2(44.0, 38.0), galley, theme::TEXT_DIM);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
