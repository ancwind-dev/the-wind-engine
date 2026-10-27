//! Свои иконки интерфейса: рисуются линиями прямо кодом на сетке 16×16, поэтому
//! чёткие при любом масштабе и не требуют файлов и чужих лицензий.
//!
//! К кнопке иконка добавляется так: в начало подписи — отступ ([`pad`]), а после
//! того как кнопка нарисована, иконка рисуется в этот отступ ([`decorate`]).

use crate::theme;
use eframe::egui::{self, Color32, Painter, Pos2, Rect, Response, Shape, Stroke, pos2, vec2};
use std::f32::consts::TAU;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    // Рабочие пространства.
    Simulation,
    MapEditor,
    Studio,
    // Инструменты редактора.
    Walls,
    Terrain,
    Ground,
    Flag,
    Select,
    Building,
    // Действия.
    Play,
    Pause,
    Step,
    Restart,
    Save,
    Open,
    New,
    Undo,
    Redo,
    // Панели и объекты.
    Soldier,
    Team,
    Network,
    Server,
    Keyboard,
    Chart,
    Cursor,
    Wrench,
    Sun,
    Check,
    Warning,
    Skeleton,
    Camera,
    Info,
    Run,
    Weapon,
}

/// Отступ в начале подписи, куда встанет иконка.
pub fn pad(text: &str) -> String {
    format!("      {text}")
}

/// Рисует иконку в отступ слева у уже нарисованного виджета.
pub fn decorate(ui: &egui::Ui, response: Response, icon: Icon) -> Response {
    let color = if response.enabled() { theme::TEXT } else { theme::TEXT_DIM };
    let r = response.rect;
    let size = 14.0;
    let at = Rect::from_min_size(pos2(r.left() + 6.0, r.center().y - size * 0.5), vec2(size, size));
    paint(ui.painter(), at, icon, color);
    response
}

/// Кнопка с иконкой.
pub fn button(ui: &mut egui::Ui, icon: Icon, text: &str) -> Response {
    let r = ui.button(pad(text));
    decorate(ui, r, icon)
}

/// Кнопка с иконкой, которую можно выключить.
pub fn button_enabled(ui: &mut egui::Ui, enabled: bool, icon: Icon, text: &str) -> Response {
    let r = ui.add_enabled(enabled, egui::Button::new(pad(text)));
    decorate(ui, r, icon)
}

/// Кнопка с иконкой во всю ширину: текст по центру, иконка — перед ним.
pub fn wide_button(ui: &mut egui::Ui, icon: Icon, text: &str) -> Response {
    let r = ui.add_sized([ui.available_width(), 22.0], egui::Button::new(text));
    decorate_centered(ui, r, icon, text)
}

/// Переключатель с иконкой (вкладка, пункт списка). `color` — цвет подписи и иконки.
pub fn selectable(ui: &mut egui::Ui, selected: bool, icon: Icon, text: &str, color: Option<Color32>) -> Response {
    let mut rich = egui::RichText::new(pad(text));
    if let Some(c) = color {
        rich = rich.color(c);
    }
    // Без переноса: иконка рассчитана на одну строку.
    let r = ui.add(egui::Button::selectable(selected, rich).wrap_mode(egui::TextWrapMode::Truncate));
    let at = Rect::from_min_size(pos2(r.rect.left() + 6.0, r.rect.center().y - 7.0), vec2(14.0, 14.0));
    paint(ui.painter(), at, icon, color.unwrap_or(theme::TEXT));
    r
}

/// Иконка перед текстом, который кнопка рисует по центру.
pub fn decorate_centered(ui: &egui::Ui, response: Response, icon: Icon, text: &str) -> Response {
    let font = egui::TextStyle::Button.resolve(ui.style());
    let width = ui.painter().layout_no_wrap(text.to_owned(), font, theme::TEXT).size().x;
    let r = response.rect;
    let color = if response.enabled() { theme::TEXT } else { theme::TEXT_DIM };
    let at = Rect::from_min_size(pos2(r.center().x - width * 0.5 - 20.0, r.center().y - 7.0), vec2(14.0, 14.0));
    paint(ui.painter(), at, icon, color);
    response
}

/// Подпись с иконкой.
pub fn label(ui: &mut egui::Ui, icon: Icon, text: &str, color: Color32) -> Response {
    let r = ui.label(egui::RichText::new(pad(text)).color(color));
    let at = Rect::from_min_size(pos2(r.rect.left() + 2.0, r.rect.center().y - 7.0), vec2(14.0, 14.0));
    paint(ui.painter(), at, icon, color);
    r
}

/// Раскрывающийся заголовок с иконкой (как в списке сцены Blender).
pub fn collapsing(ui: &mut egui::Ui, icon: Icon, text: &str, color: Color32, id: impl std::hash::Hash + std::fmt::Debug, add: impl FnOnce(&mut egui::Ui)) {
    let r = egui::CollapsingHeader::new(egui::RichText::new(pad(text)).color(color)).id_salt(id).default_open(true).show(ui, add);
    let h = r.header_response.rect;
    let x = h.left() + ui.spacing().icon_width + ui.spacing().icon_spacing + 4.0;
    paint(ui.painter(), Rect::from_min_size(pos2(x, h.center().y - 7.0), vec2(14.0, 14.0)), icon, color);
}

/// Иконка в прямоугольнике `rect` цветом `color`.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let k = rect.width() / 16.0;
    let p = |x: f32, y: f32| rect.min + vec2(x, y) * k;
    let stroke = Stroke::new((1.3 * k).max(1.0), color);
    let thin = Stroke::new((1.0 * k).max(0.8), color);
    let line = |pts: &[(f32, f32)]| {
        painter.add(Shape::line(pts.iter().map(|&(x, y)| p(x, y)).collect(), stroke));
    };
    let closed = |pts: &[(f32, f32)]| {
        painter.add(Shape::closed_line(pts.iter().map(|&(x, y)| p(x, y)).collect(), stroke));
    };
    let filled = |pts: &[(f32, f32)]| {
        painter.add(Shape::convex_polygon(pts.iter().map(|&(x, y)| p(x, y)).collect(), color, Stroke::NONE));
    };
    let rect_line = |x0: f32, y0: f32, x1: f32, y1: f32| {
        painter.rect_stroke(Rect::from_min_max(p(x0, y0), p(x1, y1)), 1.0 * k, stroke, egui::StrokeKind::Middle);
    };
    let rect_fill = |x0: f32, y0: f32, x1: f32, y1: f32| {
        painter.rect_filled(Rect::from_min_max(p(x0, y0), p(x1, y1)), 0.5 * k, color);
    };
    let circle = |x: f32, y: f32, r: f32| {
        painter.circle_stroke(p(x, y), r * k, stroke);
    };
    let dot = |x: f32, y: f32, r: f32| {
        painter.circle_filled(p(x, y), r * k, color);
    };
    let arc = |cx: f32, cy: f32, r: f32, from: f32, to: f32| -> Vec<Pos2> {
        (0..=12).map(|i| from + (to - from) * i as f32 / 12.0).map(|a| p(cx + r * a.cos(), cy + r * a.sin())).collect()
    };
    // Человечек: голова, тело, руки, ноги.
    let person = |cx: f32, s: f32| {
        dot(cx, 3.5 * s + (1.0 - s) * 8.0, 1.8 * s);
        let y = |v: f32| v * s + (1.0 - s) * 8.0;
        line(&[(cx, y(6.0)), (cx, y(10.5))]);
        line(&[(cx - 3.0 * s, y(8.0)), (cx + 3.0 * s, y(8.0))]);
        line(&[(cx - 2.5 * s, y(15.0)), (cx, y(10.5)), (cx + 2.5 * s, y(15.0))]);
    };

    match icon {
        Icon::Simulation => {
            // Два бойца.
            dot(5.0, 4.5, 1.7);
            line(&[(5.0, 7.0), (5.0, 11.0)]);
            line(&[(3.0, 15.0), (5.0, 11.0), (7.0, 15.0)]);
            dot(11.0, 4.5, 1.7);
            line(&[(11.0, 7.0), (11.0, 11.0)]);
            line(&[(9.0, 15.0), (11.0, 11.0), (13.0, 15.0)]);
            line(&[(3.0, 8.5), (14.5, 8.5)]);
        }
        Icon::MapEditor => {
            // Сложенная карта.
            closed(&[(1.5, 3.5), (5.5, 2.0), (10.5, 3.5), (14.5, 2.0), (14.5, 12.5), (10.5, 14.0), (5.5, 12.5), (1.5, 14.0)]);
            line(&[(5.5, 2.0), (5.5, 12.5)]);
            line(&[(10.5, 3.5), (10.5, 14.0)]);
        }
        Icon::Studio => {
            // Человечек в рамке, как модель в окне.
            rect_line(1.5, 1.5, 14.5, 14.5);
            dot(8.0, 5.0, 1.8);
            line(&[(8.0, 7.0), (8.0, 10.0)]);
            line(&[(5.0, 8.5), (11.0, 8.5)]);
            line(&[(5.5, 13.0), (8.0, 10.0), (10.5, 13.0)]);
        }
        Icon::Walls => {
            // Кирпичная кладка.
            rect_line(1.5, 3.0, 14.5, 13.0);
            line(&[(1.5, 6.3), (14.5, 6.3)]);
            line(&[(1.5, 9.7), (14.5, 9.7)]);
            line(&[(8.0, 3.0), (8.0, 6.3)]);
            line(&[(4.5, 6.3), (4.5, 9.7)]);
            line(&[(11.5, 6.3), (11.5, 9.7)]);
            line(&[(8.0, 9.7), (8.0, 13.0)]);
        }
        Icon::Terrain => {
            // Горы.
            line(&[(0.5, 13.5), (5.5, 5.0), (8.5, 10.0), (11.0, 6.5), (15.5, 13.5)]);
            line(&[(0.5, 13.5), (15.5, 13.5)]);
            line(&[(4.2, 7.3), (5.5, 8.3), (6.8, 7.3)]);
        }
        Icon::Ground => {
            // Шахматная клетка — покрытие земли.
            rect_line(1.5, 1.5, 14.5, 14.5);
            rect_fill(1.5, 1.5, 8.0, 8.0);
            rect_fill(8.0, 8.0, 14.5, 14.5);
        }
        Icon::Flag => {
            line(&[(3.5, 15.0), (3.5, 1.5)]);
            filled(&[(3.5, 2.0), (13.5, 5.0), (3.5, 8.5)]);
        }
        Icon::Select => {
            // Пунктирная рамка выделения.
            for (a, b) in [((2.0, 2.5), (6.0, 2.5)), ((10.0, 2.5), (14.0, 2.5)), ((2.0, 13.5), (6.0, 13.5)), ((10.0, 13.5), (14.0, 13.5))] {
                line(&[a, b]);
            }
            for (a, b) in [((2.0, 2.5), (2.0, 6.0)), ((2.0, 10.0), (2.0, 13.5)), ((14.0, 2.5), (14.0, 6.0)), ((14.0, 10.0), (14.0, 13.5))] {
                line(&[a, b]);
            }
            rect_fill(6.5, 6.5, 9.5, 9.5);
        }
        Icon::Building => {
            // Дом с крышей и дверью.
            closed(&[(2.5, 14.0), (2.5, 7.0), (8.0, 2.0), (13.5, 7.0), (13.5, 14.0)]);
            rect_line(6.5, 9.5, 9.5, 14.0);
        }
        Icon::Play => filled(&[(4.0, 2.5), (13.5, 8.0), (4.0, 13.5)]),
        Icon::Pause => {
            rect_fill(3.5, 2.5, 6.5, 13.5);
            rect_fill(9.5, 2.5, 12.5, 13.5);
        }
        Icon::Step => {
            filled(&[(2.5, 2.5), (10.0, 8.0), (2.5, 13.5)]);
            rect_fill(11.0, 2.5, 13.5, 13.5);
        }
        Icon::Restart => {
            painter.add(Shape::line(arc(8.0, 8.0, 5.5, -0.4 * TAU, 0.55 * TAU), stroke));
            let tip = p(8.0 + 5.5 * (-0.4 * TAU).cos(), 8.0 + 5.5 * (-0.4 * TAU).sin());
            painter.add(Shape::convex_polygon(vec![tip + vec2(-3.0, -1.5) * k, tip + vec2(2.5, -2.0) * k, tip + vec2(0.5, 3.0) * k], color, Stroke::NONE));
        }
        Icon::Save => {
            // Дискета.
            closed(&[(2.0, 2.0), (12.0, 2.0), (14.0, 4.0), (14.0, 14.0), (2.0, 14.0)]);
            rect_fill(4.5, 2.0, 10.5, 6.0);
            rect_line(4.5, 9.0, 11.5, 14.0);
        }
        Icon::Open => {
            // Папка.
            closed(&[(1.5, 3.0), (6.0, 3.0), (7.5, 4.8), (14.5, 4.8), (14.5, 13.5), (1.5, 13.5)]);
            line(&[(1.5, 7.0), (14.5, 7.0)]);
        }
        Icon::New => {
            // Лист с плюсом.
            closed(&[(3.0, 1.5), (10.0, 1.5), (13.0, 4.5), (13.0, 14.5), (3.0, 14.5)]);
            line(&[(10.0, 1.5), (10.0, 4.5), (13.0, 4.5)]);
            line(&[(8.0, 7.0), (8.0, 12.0)]);
            line(&[(5.5, 9.5), (10.5, 9.5)]);
        }
        Icon::Undo | Icon::Redo => {
            let flip = if icon == Icon::Redo { -1.0 } else { 1.0 };
            let x = |v: f32| 8.0 + (v - 8.0) * flip;
            let pts: Vec<Pos2> = (0..=10)
                .map(|i| 0.5 * TAU * i as f32 / 10.0)
                .map(|a| p(x(8.5 + 4.5 * a.cos()), 9.0 - 4.5 * a.sin()))
                .collect();
            painter.add(Shape::line(pts, stroke));
            filled(&[(x(1.5), 9.0), (x(6.5), 9.0), (x(4.0), 13.0)]);
        }
        Icon::Soldier => person(8.0, 1.0),
        Icon::Team => {
            // Щит.
            closed(&[(2.5, 2.5), (13.5, 2.5), (13.5, 8.0), (8.0, 14.5), (2.5, 8.0)]);
            line(&[(8.0, 2.5), (8.0, 14.5)]);
        }
        Icon::Network => {
            // Глобус.
            circle(8.0, 8.0, 6.3);
            painter.add(Shape::ellipse_stroke(p(8.0, 8.0), vec2(2.8, 6.3) * k, thin));
            line(&[(1.7, 8.0), (14.3, 8.0)]);
        }
        Icon::Server => {
            rect_line(2.0, 2.0, 14.0, 7.0);
            rect_line(2.0, 9.0, 14.0, 14.0);
            dot(11.5, 4.5, 1.0);
            dot(11.5, 11.5, 1.0);
        }
        Icon::Keyboard => {
            rect_line(1.0, 4.0, 15.0, 12.5);
            for (x, y) in [(3.5, 6.5), (6.5, 6.5), (9.5, 6.5), (12.5, 6.5)] {
                rect_fill(x - 0.8, y - 0.8, x + 0.8, y + 0.8);
            }
            line(&[(4.5, 10.0), (11.5, 10.0)]);
        }
        Icon::Chart => {
            line(&[(1.5, 1.5), (1.5, 14.5), (14.5, 14.5)]);
            rect_fill(4.0, 9.0, 6.0, 14.0);
            rect_fill(8.0, 5.0, 10.0, 14.0);
            rect_fill(12.0, 7.0, 14.0, 14.0);
        }
        Icon::Cursor => filled(&[(3.0, 1.5), (3.0, 14.0), (6.5, 10.5), (9.0, 15.0), (11.0, 14.0), (8.5, 9.5), (13.0, 9.5)]),
        Icon::Wrench => {
            line(&[(3.0, 13.0), (9.0, 7.0)]);
            painter.add(Shape::line(arc(11.0, 5.0, 3.5, 0.55 * TAU, 1.35 * TAU), stroke));
            dot(3.0, 13.0, 1.4);
        }
        Icon::Sun => {
            circle(8.0, 8.0, 3.2);
            for i in 0..8 {
                let a = TAU * i as f32 / 8.0;
                let (s, c) = a.sin_cos();
                line(&[(8.0 + c * 5.0, 8.0 + s * 5.0), (8.0 + c * 7.0, 8.0 + s * 7.0)]);
            }
        }
        Icon::Check => line(&[(2.0, 8.5), (6.0, 12.5), (14.0, 3.5)]),
        Icon::Warning => {
            closed(&[(8.0, 1.5), (15.0, 14.0), (1.0, 14.0)]);
            line(&[(8.0, 6.0), (8.0, 10.0)]);
            dot(8.0, 12.0, 0.9);
        }
        Icon::Skeleton => {
            // Палки и точки скелета.
            for &(a, b) in &[((8.0, 2.5), (8.0, 8.5)), ((8.0, 4.5), (3.5, 8.0)), ((8.0, 4.5), (12.5, 8.0)), ((8.0, 8.5), (5.0, 14.0)), ((8.0, 8.5), (11.0, 14.0))] {
                painter.line_segment([p(a.0, a.1), p(b.0, b.1)], thin);
            }
            for (x, y) in [(8.0, 2.5), (8.0, 4.5), (8.0, 8.5), (3.5, 8.0), (12.5, 8.0), (5.0, 14.0), (11.0, 14.0)] {
                dot(x, y, 1.2);
            }
        }
        Icon::Camera => {
            rect_line(1.5, 4.5, 14.5, 13.5);
            rect_fill(5.0, 2.5, 9.0, 4.5);
            circle(8.0, 9.0, 2.6);
        }
        Icon::Info => {
            circle(8.0, 8.0, 6.5);
            dot(8.0, 4.8, 1.0);
            line(&[(8.0, 7.0), (8.0, 11.8)]);
        }
        Icon::Run => {
            // Бегущий человечек.
            dot(9.5, 2.8, 1.7);
            line(&[(9.0, 5.0), (7.5, 9.5)]);
            line(&[(4.5, 6.5), (8.6, 6.0), (12.0, 8.5)]);
            line(&[(3.0, 13.5), (5.5, 11.0), (7.5, 9.5), (10.0, 12.0), (9.0, 15.0)]);
        }
        Icon::Weapon => {
            // Винтовка сбоку.
            line(&[(0.5, 7.0), (15.5, 7.0)]);
            closed(&[(2.0, 6.0), (11.0, 6.0), (11.0, 9.0), (2.0, 9.0)]);
            line(&[(6.0, 9.0), (5.0, 12.5)]);
            line(&[(2.0, 7.5), (0.5, 10.0)]);
        }
    }
}
