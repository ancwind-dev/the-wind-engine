//! Оформление движка: тёмные серые панели с холодным оттенком и тёмно-синий акцент
//! (цвет логотипа) — в духе Blender и Enfusion Workbench; оранжевое — выделенное в сцене.

use crate::i18n::t;
use crate::icons::{self, Icon};
use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontFamily, FontId, Margin, Stroke, TextStyle,
    epaint::text::{FontInsert, FontPriority, InsertFontFamily},
};

/// Шапка, строка состояния, боковая панель окна проектов.
pub const TOPBAR: Color32 = Color32::from_rgb(0x15, 0x17, 0x1a);
/// Фон областей (свойства, список объектов).
pub const REGION: Color32 = Color32::from_rgb(0x2a, 0x2d, 0x32);
/// Фон сворачиваемой панели внутри области и карточек.
pub const SUBPANEL: Color32 = Color32::from_rgb(0x33, 0x37, 0x3d);
/// Фон вьюпорта.
pub const VIEWPORT: Color32 = Color32::from_rgb(0x3a, 0x3d, 0x42);
pub const OUTLINER: Color32 = Color32::from_rgb(0x22, 0x25, 0x29);
/// Фон главного экрана и окна проектов.
pub const BACKDROP: Color32 = Color32::from_rgb(0x1c, 0x1e, 0x22);
pub const WIDGET: Color32 = Color32::from_rgb(0x46, 0x4b, 0x53);
pub const WIDGET_HOVER: Color32 = Color32::from_rgb(0x55, 0x5b, 0x64);
pub const FIELD: Color32 = Color32::from_rgb(0x19, 0x1b, 0x1f);
/// Акцент — тёмно-синий, цвет логотипа: нажатое, выбранное, главная кнопка.
pub const ACCENT: Color32 = Color32::from_rgb(0x2a, 0x52, 0x86);
/// Акцент светлее — наведение на акцентное, ссылки, полоски у карточек.
pub const ACCENT_BRIGHT: Color32 = Color32::from_rgb(0x4a, 0x80, 0xc8);
pub const SELECTED: Color32 = Color32::from_rgb(0xff, 0xa0, 0x28);
pub const TEXT: Color32 = Color32::from_rgb(0xe6, 0xe8, 0xeb);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x96, 0x9b, 0xa3);
pub const ERROR: Color32 = Color32::from_rgb(0xff, 0x6b, 0x5e);

pub const TEAM: [Color32; 2] = [
    Color32::from_rgb(0x5d, 0xa2, 0xe8),
    Color32::from_rgb(0xe8, 0x62, 0x58),
];
/// Название команды на текущем языке.
pub fn team_name(team: usize) -> &'static str {
    [t("Синие"), t("Красные")][team & 1]
}

pub fn apply(ctx: &egui::Context) {
    // Blender 4 использует Inter; на Windows ближе всего Segoe UI.
    if let Ok(bytes) = std::fs::read("C:/Windows/Fonts/segoeui.ttf") {
        ctx.add_font(FontInsert::new(
            "segoe",
            FontData::from_owned(bytes),
            vec![InsertFontFamily {
                family: FontFamily::Proportional,
                priority: FontPriority::Highest,
            }],
        ));
    }

    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, FontId::proportional(11.0)),
            (TextStyle::Body, FontId::proportional(13.0)),
            (TextStyle::Button, FontId::proportional(13.0)),
            (TextStyle::Heading, FontId::proportional(15.0)),
            (TextStyle::Monospace, FontId::monospace(12.0)),
        ]
        .into();

        let s = &mut style.spacing;
        s.item_spacing = egui::vec2(6.0, 4.0);
        s.button_padding = egui::vec2(8.0, 2.0);
        s.interact_size.y = 20.0;
        s.window_margin = Margin::same(8);
        s.menu_margin = Margin::same(4);

        let v = &mut style.visuals;
        *v = egui::Visuals::dark();
        v.override_text_color = Some(TEXT);
        v.panel_fill = REGION;
        v.window_fill = REGION;
        v.window_stroke = Stroke::new(1.0, Color32::from_rgb(0x12, 0x14, 0x17));
        v.window_corner_radius = CornerRadius::same(6);
        v.menu_corner_radius = CornerRadius::same(4);
        v.extreme_bg_color = FIELD;
        v.faint_bg_color = Color32::from_rgb(0x2d, 0x30, 0x35);
        v.selection.bg_fill = ACCENT;
        v.selection.stroke = Stroke::new(1.0, ACCENT_BRIGHT);
        v.hyperlink_color = ACCENT_BRIGHT;
        v.text_cursor.stroke = Stroke::new(2.0, ACCENT_BRIGHT);
        v.error_fg_color = ERROR;
        v.collapsing_header_frame = false;

        let radius = CornerRadius::same(4);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = REGION;
        w.noninteractive.weak_bg_fill = REGION;
        w.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x1f, 0x22, 0x26));
        w.noninteractive.corner_radius = radius;
        w.inactive.bg_fill = WIDGET;
        w.inactive.weak_bg_fill = WIDGET;
        w.inactive.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x3a, 0x3e, 0x45));
        w.inactive.corner_radius = radius;
        w.hovered.bg_fill = WIDGET_HOVER;
        w.hovered.weak_bg_fill = WIDGET_HOVER;
        w.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x6a, 0x72, 0x7d));
        w.hovered.corner_radius = radius;
        w.hovered.expansion = 0.0;
        w.active.bg_fill = ACCENT;
        w.active.weak_bg_fill = ACCENT;
        w.active.bg_stroke = Stroke::new(1.0, ACCENT_BRIGHT);
        w.active.corner_radius = radius;
        w.active.expansion = 0.0;
        w.open.bg_fill = WIDGET_HOVER;
        w.open.weak_bg_fill = WIDGET_HOVER;
        w.open.corner_radius = radius;
    });
}

/// Сворачиваемая панель с иконкой, как в редакторе свойств Blender.
pub fn panel(ui: &mut egui::Ui, icon: Icon, title: &str, open: bool, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(SUBPANEL)
        .corner_radius(4)
        .inner_margin(Margin { left: 6, right: 6, top: 4, bottom: 6 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let r = egui::CollapsingHeader::new(egui::RichText::new(icons::pad(title)).strong())
                .id_salt(title)
                .default_open(open)
                .show(ui, |ui| {
                    ui.add_space(2.0);
                    add(ui);
                });
            // Иконка — после стрелки сворачивания.
            let h = r.header_response.rect;
            let x = h.left() + ui.spacing().icon_width + ui.spacing().icon_spacing + 4.0;
            icons::paint(ui.painter(), egui::Rect::from_min_size(egui::pos2(x, h.center().y - 7.0), egui::vec2(14.0, 14.0)), icon, TEXT);
        });
    ui.add_space(4.0);
}

/// Таблица «название — значение» с подписями справа, как свойства в Blender.
pub fn props(ui: &mut egui::Ui, id: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([8.0, 4.0])
        .min_col_width(90.0)
        .show(ui, add);
}

pub fn label(ui: &mut egui::Ui, text: &str) {
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        ui.label(egui::RichText::new(text).color(TEXT_DIM));
    });
}

/// Логотип движка (белый) — текстура, загружается один раз.
pub fn logo(ctx: &egui::Context) -> egui::TextureHandle {
    let id = egui::Id::unique("wind_logo");
    if let Some(tex) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return tex;
    }
    let icon = wind_render::app_icon::for_theme(egui::Theme::Dark);
    let image = egui::ColorImage::from_rgba_unmultiplied([icon.width as usize, icon.height as usize], &icon.rgba);
    let tex = ctx.load_texture("wind_logo", image, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, tex.clone()));
    tex
}

/// Логотип размером `size` точек.
pub fn logo_image(ui: &mut egui::Ui, size: f32) -> egui::Response {
    let tex = logo(ui.ctx());
    ui.add(egui::Image::new((tex.id(), egui::vec2(size, size))))
}

/// Кнопка-логотип в шапке: на главный экран.
pub fn logo_button(ui: &mut egui::Ui) -> egui::Response {
    let tex = logo(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(egui::vec2(30.0, 20.0), egui::Sense::click());
    if response.hovered() {
        ui.painter().rect_filled(rect, 4.0, ACCENT);
    }
    let img = egui::Rect::from_center_size(rect.center(), egui::vec2(16.0, 16.0));
    ui.painter().image(tex.id(), img, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Главная кнопка (акцентная): «Открыть», «Создать».
pub fn primary_button(ui: &mut egui::Ui, text: &str) -> egui::Response {
    let button = egui::Button::new(egui::RichText::new(text).color(Color32::WHITE).strong())
        .fill(ACCENT)
        .stroke(Stroke::new(1.0, ACCENT_BRIGHT))
        .min_size(egui::vec2(96.0, 28.0));
    ui.add(button)
}

/// Крупная кнопка с иконкой во всю ширину — для боковой панели окна проектов.
pub fn side_button(ui: &mut egui::Ui, icon: Icon, text: &str, active: bool) -> egui::Response {
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, 34.0), egui::Sense::click());
    let fill = if active {
        ACCENT
    } else if response.hovered() {
        Color32::from_rgb(0x26, 0x2a, 0x30)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, 5.0, fill);
    let at = egui::Rect::from_min_size(egui::pos2(rect.left() + 12.0, rect.center().y - 8.0), egui::vec2(16.0, 16.0));
    icons::paint(ui.painter(), at, icon, TEXT);
    ui.painter().text(
        egui::pos2(rect.left() + 38.0, rect.center().y),
        egui::Align2::LEFT_CENTER,
        text,
        FontId::proportional(14.0),
        TEXT,
    );
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}
