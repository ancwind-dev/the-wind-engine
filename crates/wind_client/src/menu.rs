//! Главное меню, меню паузы (Esc), настройки и подключение к серверу. Рисуется
//! поверх боя: в главном меню за ним идёт бой ИИ, в паузе — своя игра.

use crate::settings::{self, Settings};
use eframe::egui::{self, Align2, Color32, CornerRadius, Frame, Margin, RichText, Stroke, vec2};
use std::path::Path;

const PANEL: Color32 = Color32::from_rgba_premultiplied(14, 16, 18, 215);
const ACCENT: Color32 = Color32::from_rgb(0x5d, 0xa2, 0xe8);
const DIM: Color32 = Color32::from_rgb(0xb0, 0xb0, 0xb0);
const ERROR: Color32 = Color32::from_rgb(0xff, 0x7a, 0x6a);
const BUTTON: egui::Vec2 = vec2(300.0, 40.0);
/// Ширина колонки подписей в настройках — чтобы поля двух таблиц стояли ровно.
const LABEL: f32 = 112.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Page {
    Main,
    Settings,
    Connect,
}

/// Что выбрал игрок.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    PlayLocal,
    Spectate,
    /// Подключиться к `settings.server` под именем `settings.name`.
    Connect,
    Resume,
    ToMainMenu,
    /// Ушли со страницы настроек — пора сохранить.
    SettingsClosed,
    Quit,
}

pub struct Menu {
    pub open: bool,
    pub page: Page,
    /// Ошибка последнего действия (не подключились, не сохранились).
    pub error: Option<String>,
    /// Карты из `data/maps`.
    maps: Vec<String>,
}

/// Папки карт в `data/maps` (в каждой — `map.ron`).
pub fn list_maps(dir: &Path) -> Vec<String> {
    let mut maps: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().join("map.ron").exists())
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    maps.sort();
    maps
}

impl Menu {
    pub fn new(open: bool, maps: Vec<String>) -> Menu {
        Menu { open, page: Page::Main, error: None, maps }
    }

    /// Esc: со страницы — назад, в паузе — продолжить, в игре — пауза.
    pub fn escape(&mut self, in_match: bool) -> Action {
        if !self.open {
            self.open = true;
            self.page = Page::Main;
            return Action::None;
        }
        match self.page {
            Page::Settings => {
                self.page = Page::Main;
                Action::SettingsClosed
            }
            Page::Connect => {
                self.page = Page::Main;
                Action::None
            }
            Page::Main if in_match => Action::Resume,
            Page::Main => Action::None,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, settings: &mut Settings, in_match: bool) -> Action {
        if !self.open {
            return Action::None;
        }
        // Затемняем бой, чтобы меню читалось.
        let screen = ctx.content_rect();
        egui::Area::new(egui::Id::unique("menu-shade"))
            .order(egui::Order::Background)
            .fixed_pos(screen.min)
            .interactable(false)
            .show(ctx, |ui| ui.painter().rect_filled(screen, 0, Color32::from_black_alpha(if in_match { 120 } else { 60 })));

        let mut action = Action::None;
        egui::Area::new(egui::Id::unique("menu")).order(egui::Order::Foreground).anchor(Align2::CENTER_CENTER, vec2(0.0, 0.0)).show(ctx, |ui| {
            Frame::new()
                .fill(PANEL)
                .stroke(Stroke::new(1.0, Color32::from_white_alpha(18)))
                .corner_radius(CornerRadius::same(10))
                .inner_margin(Margin::same(26))
                .show(ui, |ui| {
                    ui.set_width(BUTTON.x);
                    ui.spacing_mut().item_spacing = vec2(8.0, 10.0);
                    action = match self.page {
                        Page::Main => self.main_page(ui, in_match),
                        Page::Settings => self.settings_page(ui, settings),
                        Page::Connect => self.connect_page(ui, settings),
                    };
                    if let Some(e) = &self.error {
                        ui.add_space(4.0);
                        ui.label(RichText::new(e).color(ERROR));
                    }
                });
        });
        if action != Action::None {
            self.error = None;
        }
        action
    }

    fn main_page(&mut self, ui: &mut egui::Ui, in_match: bool) -> Action {
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("THE WIND ENGINE").size(26.0).strong().color(Color32::WHITE));
            ui.label(RichText::new(if in_match { "Пауза" } else { "тактический бой сверху" }).color(DIM));
        });
        ui.add_space(8.0);
        let mut action = Action::None;
        if in_match {
            if big_button(ui, "Продолжить") {
                action = Action::Resume;
            }
        } else {
            if big_button(ui, "Играть против ИИ") {
                action = Action::PlayLocal;
            }
            if big_button(ui, "Игра по сети") {
                self.page = Page::Connect;
            }
            if big_button(ui, "Наблюдать бой") {
                action = Action::Spectate;
            }
        }
        if big_button(ui, "Настройки") {
            self.page = Page::Settings;
        }
        if in_match && big_button(ui, "В главное меню") {
            action = Action::ToMainMenu;
        }
        if big_button(ui, "Выход") {
            action = Action::Quit;
        }
        if in_match {
            ui.add_space(2.0);
            ui.vertical_centered(|ui| ui.label(RichText::new("Esc — вернуться в бой").small().color(DIM)));
        }
        action
    }

    fn settings_page(&mut self, ui: &mut egui::Ui, s: &mut Settings) -> Action {
        heading(ui, "Настройки");
        egui::Grid::new("settings").num_columns(2).min_col_width(LABEL).spacing(vec2(12.0, 10.0)).show(ui, |ui| {
            ui.label("Имя");
            ui.add(egui::TextEdit::singleline(&mut s.name).char_limit(24).desired_width(170.0));
            ui.end_row();

            ui.label("Громкость");
            let mut percent = (s.volume * 100.0).round() as u32;
            if ui.add(egui::Slider::new(&mut percent, 0..=100).suffix(" %")).changed() {
                s.volume = percent as f32 / 100.0;
            }
            ui.end_row();
        });
        ui.add_space(4.0);
        ui.label(RichText::new("Бой против ИИ").strong());
        ui.label(RichText::new("Применяется в следующем бою").small().color(DIM));
        egui::Grid::new("battle").num_columns(2).min_col_width(LABEL).spacing(vec2(12.0, 10.0)).show(ui, |ui| {
            ui.label("Солдат в команде");
            ui.add(egui::Slider::new(&mut s.soldiers_per_team, settings::SOLDIERS).logarithmic(true));
            ui.end_row();

            ui.label("Карта");
            let shown = if s.map_file.is_empty() { "Случайная".to_string() } else { s.map_file.clone() };
            egui::ComboBox::from_id_salt("map").selected_text(shown).width(170.0).show_ui(ui, |ui| {
                ui.selectable_value(&mut s.map_file, String::new(), "Случайная");
                for m in &self.maps {
                    ui.selectable_value(&mut s.map_file, m.clone(), m);
                }
            });
            ui.end_row();

            ui.label("Размер карты");
            ui.add_enabled(s.map_file.is_empty(), egui::Slider::new(&mut s.map_size, settings::MAP_SIZE).step_by(64.0).suffix(" м"));
            ui.end_row();
        });
        ui.add_space(6.0);
        if big_button(ui, "Назад") {
            self.page = Page::Main;
            return Action::SettingsClosed;
        }
        Action::None
    }

    fn connect_page(&mut self, ui: &mut egui::Ui, s: &mut Settings) -> Action {
        heading(ui, "Игра по сети");
        let address = egui::Grid::new("connect")
            .num_columns(2)
            .spacing(vec2(12.0, 10.0))
            .show(ui, |ui| {
                ui.label("Сервер");
                let address = ui.add(egui::TextEdit::singleline(&mut s.server).hint_text("адрес:порт").desired_width(190.0));
                ui.end_row();
                ui.label("Имя");
                ui.add(egui::TextEdit::singleline(&mut s.name).char_limit(24).desired_width(190.0));
                ui.end_row();
                ui.label("Пароль");
                ui.add(egui::TextEdit::singleline(&mut s.password).password(true).hint_text("если сервер просит").desired_width(190.0));
                ui.end_row();
                address
            })
            .inner;
        // Enter в поле адреса — сразу подключиться.
        let enter = address.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        ui.add_space(6.0);
        let mut action = Action::None;
        if big_button(ui, "Подключиться") || enter {
            action = if s.server.trim().is_empty() {
                self.error = Some("Укажи адрес сервера".into());
                Action::None
            } else {
                Action::Connect
            };
        }
        if big_button(ui, "Назад") {
            self.page = Page::Main;
        }
        action
    }
}

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.vertical_centered(|ui| ui.label(RichText::new(text).size(20.0).strong().color(Color32::WHITE)));
    ui.add_space(4.0);
}

fn big_button(ui: &mut egui::Ui, text: &str) -> bool {
    let button = egui::Button::new(RichText::new(text).size(17.0).color(Color32::WHITE))
        .fill(Color32::from_white_alpha(14))
        .stroke(Stroke::new(1.0, ACCENT.gamma_multiply(0.5)))
        .corner_radius(CornerRadius::same(6));
    ui.add_sized(BUTTON, button).clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_goes_back_then_resumes() {
        let mut m = Menu::new(false, Vec::new());
        // В игре: Esc — пауза, ещё раз — продолжить.
        assert_eq!(m.escape(true), Action::None);
        assert!(m.open);
        assert_eq!(m.escape(true), Action::Resume);
        // Из настроек Esc возвращает на главную страницу и просит сохранить.
        m.page = Page::Settings;
        assert_eq!(m.escape(true), Action::SettingsClosed);
        assert_eq!(m.page, Page::Main);
        // В главном меню (боя нет) Esc на главной странице ничего не закрывает.
        let mut m = Menu::new(true, Vec::new());
        assert_eq!(m.escape(false), Action::None);
        assert!(m.open);
    }

    #[test]
    fn maps_are_found_in_data() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/maps");
        assert!(list_maps(&dir).contains(&"пример".to_string()));
    }
}
