//! Главный экран проекта: выбор редактора, карты проекта и сводка по данным.
//! Сюда попадаем, открыв проект, и возвращаемся кнопкой-логотипом из любого редактора.

use crate::i18n::t;
use crate::icons::{self, Icon};
use crate::lab::Workspace;
use crate::project::Project;
use crate::theme;
use eframe::egui::{self, Color32, FontId, Margin, RichText, Stroke};

pub enum HubAction {
    Open(Workspace),
    /// Открыть карту в редакторе карт.
    OpenMap(String),
    /// Запустить игру этого проекта.
    Play,
    ShowFolder,
    ChangeProject,
    Settings,
}

/// Что показываем о проекте; считается при открытии, а не каждый кадр.
pub struct Hub {
    maps: Vec<String>,
    soldiers: usize,
    props: usize,
    clips: usize,
    /// Сообщение после действия (например, игра не запустилась).
    pub message: Option<(String, Color32)>,
}

impl Hub {
    pub fn new() -> Self {
        Hub { maps: Vec::new(), soldiers: 0, props: 0, clips: 0, message: None }
    }

    pub fn refresh(&mut self, project: &Project) {
        self.maps = project.maps();
        self.soldiers = project.count_files("models/soldiers", "xml");
        self.props = project.count_files("models/props", "glb");
        self.clips = project.count_files("animations", "ron");
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, project: &Project) -> Option<HubAction> {
        let mut action = None;
        self.top_bar(ui, project, &mut action);
        egui::Panel::right("hub_project")
            .exact_size(300.0)
            .resizable(false)
            .frame(egui::Frame::new().fill(theme::OUTLINER).inner_margin(Margin::same(18)))
            .show(ui, |ui| self.summary(ui, project, &mut action));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(theme::BACKDROP).inner_margin(Margin { left: 36, right: 36, top: 30, bottom: 24 }))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink(false).show(ui, |ui| self.content(ui, project, &mut action));
            });
        action
    }

    fn top_bar(&mut self, ui: &mut egui::Ui, project: &Project, action: &mut Option<HubAction>) {
        egui::Panel::top("hub_top")
            .frame(egui::Frame::new().fill(theme::TOPBAR).inner_margin(Margin::symmetric(8, 6)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let _ = theme::logo_button(ui);
                    ui.label(RichText::new(project.name()).strong());
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if icons::button(ui, Icon::Gear, t("Настройки")).clicked() {
                            *action = Some(HubAction::Settings);
                        }
                        if icons::button(ui, Icon::Open, t("Сменить проект")).clicked() {
                            *action = Some(HubAction::ChangeProject);
                        }
                    });
                });
            });
    }

    fn content(&mut self, ui: &mut egui::Ui, project: &Project, action: &mut Option<HubAction>) {
        ui.label(RichText::new(project.name()).font(FontId::proportional(28.0)).strong());
        if !project.manifest.description.is_empty() {
            ui.add_space(2.0);
            ui.label(RichText::new(&project.manifest.description).color(theme::TEXT_DIM));
        }
        ui.add_space(26.0);
        section(ui, t("Редакторы"));
        let tiles = [
            (Workspace::Simulation, Icon::Simulation, t("Лаборатория"), t("Бой ИИ на любой карте, свой сервер и игра по сети, замеры по фазам.")),
            (Workspace::Map, Icon::MapEditor, t("Редактор карт"), t("Рельеф, земля, стены, дома, предметы и точки захвата.")),
            (Workspace::Studio, Icon::Studio, t("Студия персонажей"), t("Модели солдат и оружие в руках, движения и редактор анимаций.")),
        ];
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(14.0, 14.0);
            for (ws, icon, title, about) in tiles {
                if tile(ui, icon, title, about, false).clicked() {
                    *action = Some(HubAction::Open(ws));
                }
            }
            if tile(ui, Icon::Run, t("Играть"), t("Запустить игру этого проекта в отдельном окне."), true).clicked() {
                *action = Some(HubAction::Play);
            }
        });
        if let Some((text, color)) = &self.message {
            ui.add_space(8.0);
            ui.label(RichText::new(text).color(*color));
        }
        ui.add_space(30.0);
        section(ui, t("Карты проекта"));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(10.0, 10.0);
            for name in &self.maps {
                if map_chip(ui, Icon::MapEditor, name).on_hover_text(t("Открыть в редакторе карт")).clicked() {
                    *action = Some(HubAction::OpenMap(name.clone()));
                }
            }
            if map_chip(ui, Icon::New, t("Новая карта")).clicked() {
                *action = Some(HubAction::Open(Workspace::Map));
            }
        });
        if self.maps.is_empty() {
            ui.add_space(4.0);
            ui.label(RichText::new(t("Карт пока нет — бой идёт на процедурной карте.")).small().color(theme::TEXT_DIM));
        }
    }

    fn summary(&mut self, ui: &mut egui::Ui, project: &Project, action: &mut Option<HubAction>) {
        ui.label(RichText::new(t("Проект")).font(FontId::proportional(16.0)).strong());
        ui.add_space(10.0);
        ui.label(RichText::new(t("Папка")).small().color(theme::TEXT_DIM));
        ui.add(egui::Label::new(RichText::new(project.root.display().to_string()).small()).wrap());
        ui.add_space(12.0);
        let rows = [
            (Icon::MapEditor, t("Карты"), self.maps.len()),
            (Icon::Soldier, t("Модели солдат"), self.soldiers),
            (Icon::Building, t("Предметы и дома"), self.props),
            (Icon::Run, t("Клипы анимации"), self.clips),
        ];
        for (icon, label, n) in rows {
            ui.horizontal(|ui| {
                icons::label(ui, icon, label, theme::TEXT);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(n.to_string()).strong());
                });
            });
            ui.add_space(2.0);
        }
        ui.add_space(14.0);
        if icons::wide_button(ui, Icon::Folder, t("Открыть папку проекта")).clicked() {
            *action = Some(HubAction::ShowFolder);
        }
        ui.add_space(18.0);
        ui.label(RichText::new(t("Подсказка")).small().color(theme::TEXT_DIM));
        ui.add(
            egui::Label::new(
                RichText::new(t("Логотип слева в шапке любого редактора возвращает на этот экран. Данные (data) подхватываются на лету — правь файлы, движок обновится сам."))
                    .small()
                    .color(theme::TEXT_DIM),
            )
            .wrap(),
        );
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            ui.label(RichText::new(concat!("The Wind Engine ", env!("CARGO_PKG_VERSION"))).small().color(theme::TEXT_DIM));
        });
    }
}

/// Заголовок раздела с тонкой линией.
fn section(ui: &mut egui::Ui, title: &str) {
    ui.label(RichText::new(title.to_uppercase()).small().strong().color(theme::TEXT_DIM));
    let y = ui.cursor().top() + 2.0;
    let x = ui.max_rect().x_range();
    ui.painter().hline(x, y, Stroke::new(1.0, Color32::from_rgb(0x2c, 0x30, 0x36)));
    ui.add_space(12.0);
}

/// Плитка редактора: иконка в круге, название, описание; при наведении — синяя полоса сверху.
fn tile(ui: &mut egui::Ui, icon: Icon, title: &str, about: &str, accent: bool) -> egui::Response {
    let size = egui::vec2(236.0, 164.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let hovered = response.hovered();
    let p = ui.painter();
    let fill = match (accent, hovered) {
        (true, false) => theme::ACCENT,
        (true, true) => Color32::from_rgb(0x33, 0x62, 0x9e),
        (false, false) => theme::SUBPANEL,
        (false, true) => Color32::from_rgb(0x3b, 0x40, 0x47),
    };
    p.rect_filled(rect, 8.0, fill);
    if hovered && !accent {
        let bar = egui::Rect::from_min_size(rect.min, egui::vec2(size.x, 3.0));
        p.rect_filled(bar, egui::CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 }, theme::ACCENT_BRIGHT);
    }
    let center = rect.min + egui::vec2(38.0, 40.0);
    let ring = if accent { Color32::from_white_alpha(40) } else { theme::ACCENT };
    p.circle_filled(center, 22.0, ring);
    icons::paint(p, egui::Rect::from_center_size(center, egui::vec2(24.0, 24.0)), icon, Color32::WHITE);
    p.text(rect.min + egui::vec2(18.0, 76.0), egui::Align2::LEFT_TOP, title, FontId::proportional(17.0), Color32::WHITE);
    let dim = if accent { Color32::from_white_alpha(200) } else { theme::TEXT_DIM };
    let galley = p.layout(about.to_owned(), FontId::proportional(12.5), dim, size.x - 36.0);
    p.galley(rect.min + egui::vec2(18.0, 102.0), galley, dim);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Карта проекта — «таблетка» с иконкой.
fn map_chip(ui: &mut egui::Ui, icon: Icon, name: &str) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(name.to_owned(), FontId::proportional(13.5), theme::TEXT);
    let size = egui::vec2(galley.size().x + 52.0, 38.0);
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    let p = ui.painter();
    p.rect_filled(rect, 6.0, if response.hovered() { theme::WIDGET } else { theme::SUBPANEL });
    if response.hovered() {
        p.rect_stroke(rect, 6.0, Stroke::new(1.0, theme::ACCENT_BRIGHT), egui::StrokeKind::Inside);
    }
    icons::paint(p, egui::Rect::from_min_size(rect.min + egui::vec2(14.0, 11.0), egui::vec2(16.0, 16.0)), icon, theme::ACCENT_BRIGHT);
    p.galley(rect.min + egui::vec2(38.0, (size.y - galley.size().y) * 0.5), galley, theme::TEXT);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
