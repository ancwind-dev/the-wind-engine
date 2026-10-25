//! Оформление в духе Blender: тёмно-серые панели, синий акцент, оранжевое выделение.

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontFamily, FontId, Margin, Stroke, TextStyle,
    epaint::text::{FontInsert, FontPriority, InsertFontFamily},
};

/// Шапка и строка состояния.
pub const TOPBAR: Color32 = Color32::from_rgb(0x18, 0x18, 0x18);
/// Фон областей (свойства, список объектов).
pub const REGION: Color32 = Color32::from_rgb(0x30, 0x30, 0x30);
/// Фон сворачиваемой панели внутри области.
pub const SUBPANEL: Color32 = Color32::from_rgb(0x3d, 0x3d, 0x3d);
/// Фон вьюпорта.
pub const VIEWPORT: Color32 = Color32::from_rgb(0x3d, 0x3d, 0x3d);
pub const OUTLINER: Color32 = Color32::from_rgb(0x28, 0x28, 0x28);
pub const WIDGET: Color32 = Color32::from_rgb(0x54, 0x54, 0x54);
pub const WIDGET_HOVER: Color32 = Color32::from_rgb(0x65, 0x65, 0x65);
pub const FIELD: Color32 = Color32::from_rgb(0x1d, 0x1d, 0x1d);
pub const ACCENT: Color32 = Color32::from_rgb(0x47, 0x72, 0xb3);
pub const SELECTED: Color32 = Color32::from_rgb(0xff, 0xa0, 0x28);
pub const TEXT: Color32 = Color32::from_rgb(0xe6, 0xe6, 0xe6);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x9a, 0x9a, 0x9a);
pub const ERROR: Color32 = Color32::from_rgb(0xff, 0x6b, 0x5e);

pub const TEAM: [Color32; 2] = [
    Color32::from_rgb(0x5d, 0xa2, 0xe8),
    Color32::from_rgb(0xe8, 0x62, 0x58),
];
pub const TEAM_NAME: [&str; 2] = ["Синие", "Красные"];

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
        v.window_stroke = Stroke::new(1.0, Color32::from_gray(0x24));
        v.window_corner_radius = CornerRadius::same(6);
        v.menu_corner_radius = CornerRadius::same(4);
        v.extreme_bg_color = FIELD;
        v.faint_bg_color = Color32::from_gray(0x2b);
        v.selection.bg_fill = ACCENT;
        v.selection.stroke = Stroke::new(1.0, TEXT);
        v.hyperlink_color = ACCENT;
        v.error_fg_color = ERROR;
        v.collapsing_header_frame = false;

        let radius = CornerRadius::same(4);
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = REGION;
        w.noninteractive.weak_bg_fill = REGION;
        w.noninteractive.bg_stroke = Stroke::new(1.0, Color32::from_gray(0x24));
        w.noninteractive.corner_radius = radius;
        w.inactive.bg_fill = WIDGET;
        w.inactive.weak_bg_fill = WIDGET;
        w.inactive.bg_stroke = Stroke::new(1.0, Color32::from_gray(0x3d));
        w.inactive.corner_radius = radius;
        w.hovered.bg_fill = WIDGET_HOVER;
        w.hovered.weak_bg_fill = WIDGET_HOVER;
        w.hovered.bg_stroke = Stroke::new(1.0, Color32::from_gray(0x70));
        w.hovered.corner_radius = radius;
        w.hovered.expansion = 0.0;
        w.active.bg_fill = ACCENT;
        w.active.weak_bg_fill = ACCENT;
        w.active.bg_stroke = Stroke::new(1.0, ACCENT);
        w.active.corner_radius = radius;
        w.active.expansion = 0.0;
        w.open.bg_fill = WIDGET_HOVER;
        w.open.weak_bg_fill = WIDGET_HOVER;
        w.open.corner_radius = radius;
    });
}

/// Сворачиваемая панель, как в редакторе свойств Blender.
pub fn panel(ui: &mut egui::Ui, title: &str, open: bool, add: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new()
        .fill(SUBPANEL)
        .corner_radius(4)
        .inner_margin(Margin { left: 6, right: 6, top: 4, bottom: 6 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::CollapsingHeader::new(egui::RichText::new(title).strong())
                .default_open(open)
                .show(ui, |ui| {
                    ui.add_space(2.0);
                    add(ui);
                });
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
