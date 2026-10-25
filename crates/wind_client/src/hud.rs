//! HUD поверх 3D: состояние солдата, счёт и точки, лента убийств, прицел, FPS.

use crate::source::View;
use eframe::egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Stroke, pos2, vec2};

const TEAM: [Color32; 2] = [Color32::from_rgb(0x5d, 0xa2, 0xe8), Color32::from_rgb(0xe8, 0x62, 0x58)];
const PANEL: Color32 = Color32::from_black_alpha(150);
const TEXT: Color32 = Color32::from_rgb(0xee, 0xee, 0xee);
const DIM: Color32 = Color32::from_rgb(0xb0, 0xb0, 0xb0);

pub fn draw(painter: &Painter, rect: Rect, view: &View, status: Option<String>, spectator: bool) {
    // Счёт и точки — сверху по центру.
    let top = rect.center_top() + vec2(0.0, 14.0);
    let n = view.owners.len();
    let width = 120.0 + n as f32 * 26.0;
    painter.rect_filled(Rect::from_center_size(top + vec2(0.0, 14.0), vec2(width, 30.0)), 6, PANEL);
    painter.text(top + vec2(-width / 2.0 + 12.0, 14.0), Align2::LEFT_CENTER, view.score[0].to_string(), FontId::proportional(16.0), TEAM[0]);
    painter.text(top + vec2(width / 2.0 - 12.0, 14.0), Align2::RIGHT_CENTER, view.score[1].to_string(), FontId::proportional(16.0), TEAM[1]);
    for (i, owner) in view.owners.iter().enumerate() {
        let x = (i as f32 - (n as f32 - 1.0) / 2.0) * 26.0;
        let c = top + vec2(x, 14.0);
        let color = owner.map_or(Color32::from_gray(0x90), |t| TEAM[t as usize & 1]);
        painter.rect_filled(Rect::from_center_size(c, vec2(20.0, 20.0)), 3, color.gamma_multiply(0.85));
        painter.text(c, Align2::CENTER_CENTER, (b'A' + i as u8) as char, FontId::proportional(13.0), Color32::BLACK);
    }

    // Лента убийств — справа сверху.
    for (i, (text, team)) in view.kill_feed.iter().enumerate() {
        let at = rect.right_top() + vec2(-14.0, 14.0 + i as f32 * 18.0);
        painter.text(at, Align2::RIGHT_TOP, text, FontId::proportional(13.0), TEAM[*team as usize & 1]);
    }

    if let Some(status) = status {
        painter.rect_filled(Rect::from_center_size(rect.center(), vec2(420.0, 44.0)), 8, PANEL);
        painter.text(rect.center(), Align2::CENTER_CENTER, status, FontId::proportional(16.0), TEXT);
        return;
    }
    if spectator {
        return;
    }

    // Свой солдат — снизу по центру.
    let me = &view.me;
    let bar = Rect::from_center_size(rect.center_bottom() - vec2(0.0, 34.0), vec2(420.0, 40.0));
    painter.rect_filled(bar, 8, PANEL);
    if me.alive {
        // Полоска здоровья.
        let hp = Rect::from_min_size(bar.left_center() + vec2(14.0, -5.0), vec2(110.0, 10.0));
        painter.rect_filled(hp, 3, Color32::from_gray(40));
        let mut fill = hp;
        fill.set_width(hp.width() * me.health.clamp(0.0, 1.0));
        painter.rect_filled(fill, 3, if me.health > 0.5 { Color32::from_rgb(110, 190, 90) } else { Color32::from_rgb(220, 90, 70) });
        let ammo = if me.reloading { "перезарядка…".to_string() } else { format!("{} / {}", me.ammo, me.magazine) };
        painter.text(bar.center() + vec2(10.0, 0.0), Align2::CENTER_CENTER, ammo, FontId::proportional(18.0), TEXT);
        painter.text(
            bar.right_center() - vec2(14.0, 0.0),
            Align2::RIGHT_CENTER,
            format!("гранат {} · убийств {}", me.grenades, me.kills),
            FontId::proportional(14.0),
            DIM,
        );
    } else {
        painter.text(
            bar.center(),
            Align2::CENTER_CENTER,
            format!("Убит. Появление через {:.1} с", me.respawn_in),
            FontId::proportional(16.0),
            Color32::from_rgb(240, 120, 100),
        );
    }
}

pub fn crosshair(painter: &Painter, at: Pos2) {
    let stroke = Stroke::new(1.5, Color32::from_white_alpha(220));
    for (a, b) in [(vec2(-10.0, 0.0), vec2(-4.0, 0.0)), (vec2(4.0, 0.0), vec2(10.0, 0.0)), (vec2(0.0, -10.0), vec2(0.0, -4.0)), (vec2(0.0, 4.0), vec2(0.0, 10.0))] {
        painter.line_segment([at + a, at + b], stroke);
    }
}

pub fn perf(painter: &Painter, rect: Rect, fps: &str, debug: Option<Vec<String>>) {
    let at = rect.left_top() + vec2(12.0, 10.0);
    painter.text(at, Align2::LEFT_TOP, fps, FontId::monospace(12.0), Color32::from_black_alpha(200));
    if let Some(lines) = debug {
        let h = 16.0 * lines.len() as f32 + 12.0;
        let panel = Rect::from_min_size(at + vec2(-4.0, 20.0), vec2(430.0, h));
        painter.rect_filled(panel, 6, PANEL);
        for (i, line) in lines.iter().enumerate() {
            painter.text(pos2(panel.left() + 8.0, panel.top() + 6.0 + i as f32 * 16.0), Align2::LEFT_TOP, line, FontId::monospace(12.0), TEXT);
        }
    }
}
