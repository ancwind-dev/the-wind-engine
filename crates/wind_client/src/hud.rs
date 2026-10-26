//! HUD поверх 3D: состояние солдата, счёт и точки, лента убийств, прицел, FPS.

use crate::source::View;
use eframe::egui::{self, Align2, Color32, FontId, Painter, Pos2, Rect, Stroke, pos2, vec2};

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
    let bar = Rect::from_center_size(rect.center_bottom() - vec2(0.0, 34.0), vec2(480.0, 40.0));
    painter.rect_filled(bar, 8, PANEL);
    if me.alive {
        // Полоска здоровья.
        let hp = Rect::from_min_size(bar.left_center() + vec2(14.0, -5.0), vec2(110.0, 10.0));
        painter.rect_filled(hp, 3, Color32::from_gray(40));
        let mut fill = hp;
        fill.set_width(hp.width() * me.health.clamp(0.0, 1.0));
        painter.rect_filled(fill, 3, if me.health > 0.5 { Color32::from_rgb(110, 190, 90) } else { Color32::from_rgb(220, 90, 70) });
        let ammo = if me.reloading { format!("{} · перезарядка…", me.weapon) } else { format!("{}  {} / {}", me.weapon, me.ammo, me.magazine) };
        // Оружие и патроны — сразу за полоской здоровья, слева направо.
        painter.text(hp.right_center() + vec2(16.0, 0.0), Align2::LEFT_CENTER, ammo, FontId::proportional(18.0), TEXT);
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

fn clock(seconds: f32) -> String {
    let s = seconds.max(0.0) as u32;
    format!("{}:{:02}", s / 60, s % 60)
}

/// Раунд: время под счётом, полоса удержания всех точек, а после конца — итоги.
pub fn round(painter: &Painter, rect: Rect, view: &View) {
    let r = &view.round;
    let top = rect.center_top() + vec2(0.0, 52.0);
    if r.round.playing() {
        let left = if r.time_limit > 0.0 { clock(r.time_limit - r.round.time) } else { clock(r.round.time) };
        painter.text(top, Align2::CENTER_CENTER, format!("Раунд {} · {left}", r.round.number), FontId::proportional(13.0), TEXT);
        // Кто держит все точки — полоса и сколько осталось до победы.
        if let Some(team) = (0..2).find(|&t| r.round.hold[t] > 0.0) {
            let k = (r.round.hold[team] / r.hold_time.max(0.1)).clamp(0.0, 1.0);
            let bar = Rect::from_center_size(top + vec2(0.0, 20.0), vec2(300.0, 8.0));
            painter.rect_filled(bar, 3, Color32::from_black_alpha(140));
            let mut fill = bar;
            fill.set_width(bar.width() * k);
            painter.rect_filled(fill, 3, TEAM[team]);
            let who = &r.team_names[team];
            painter.text(
                bar.center_bottom() + vec2(0.0, 10.0),
                Align2::CENTER_CENTER,
                format!("{who} держат все точки — победа через {}", clock(r.hold_time - r.round.hold[team])),
                FontId::proportional(13.0),
                TEAM[team],
            );
        }
        return;
    }
    // Итоги раунда.
    let wind_sim::round::Phase::Over { winner, reason } = r.round.phase else { return };
    let panel = Rect::from_center_size(rect.center() - vec2(0.0, 60.0), vec2(460.0, 190.0));
    painter.rect_filled(panel, 10, Color32::from_black_alpha(190));
    let (title, color) = match (winner, r.own_team) {
        (None, _) => ("Ничья".to_string(), TEXT),
        (Some(w), Some(me)) if w == me => ("Победа!".to_string(), TEAM[w as usize & 1]),
        (Some(w), Some(_)) => ("Поражение".to_string(), TEAM[w as usize & 1]),
        (Some(w), None) => (format!("Победили {}", r.team_names[w as usize & 1]), TEAM[w as usize & 1]),
    };
    painter.text(panel.center_top() + vec2(0.0, 36.0), Align2::CENTER_CENTER, title, FontId::proportional(32.0), color);
    let why = match (winner, reason) {
        (Some(w), wind_sim::round::EndReason::AllPoints) => format!("{} удержали все точки", r.team_names[w as usize & 1]),
        (Some(w), wind_sim::round::EndReason::TimeUp) => format!("Время вышло — у {} больше точек", r.team_names[w as usize & 1]),
        (None, _) => "Время вышло — точек поровну".to_string(),
    };
    painter.text(panel.center_top() + vec2(0.0, 72.0), Align2::CENTER_CENTER, why, FontId::proportional(15.0), DIM);
    let score = format!("Убито: {} {} — {} {}", r.team_names[0], view.score[0], view.score[1], r.team_names[1]);
    painter.text(panel.center_top() + vec2(0.0, 104.0), Align2::CENTER_CENTER, score, FontId::proportional(15.0), TEXT);
    if view.has_me {
        painter.text(panel.center_top() + vec2(0.0, 130.0), Align2::CENTER_CENTER, format!("Ты убил: {}", view.me.kills), FontId::proportional(15.0), TEXT);
    }
    painter.text(
        panel.center_bottom() - vec2(0.0, 22.0),
        Align2::CENTER_CENTER,
        format!("Новый раунд через {:.0} с", (r.intermission - r.round.time).max(0.0).ceil()),
        FontId::proportional(14.0),
        DIM,
    );
}

/// Пока убит — где появиться: база или своя точка. Клавиши 1–6.
pub fn spawn_choice(painter: &Painter, rect: Rect, view: &View, chosen: u8) {
    let Some(team) = view.round.own_team else { return };
    let mut items = vec![("1 — база".to_string(), true)];
    for (i, owner) in view.owners.iter().enumerate().take(5) {
        items.push((format!("{} — {}", i + 2, (b'A' + i as u8) as char), *owner == Some(team)));
    }
    let w = 96.0;
    let total = w * items.len() as f32;
    let y = rect.bottom() - 96.0;
    painter.text(pos2(rect.center().x, y - 20.0), Align2::CENTER_CENTER, "Где появиться", FontId::proportional(13.0), DIM);
    for (k, (label, ours)) in items.iter().enumerate() {
        let c = pos2(rect.center().x - total / 2.0 + w * (k as f32 + 0.5), y);
        let r = Rect::from_center_size(c, vec2(w - 8.0, 26.0));
        let selected = k as u8 == chosen;
        let fill = if selected { TEAM[team as usize & 1].gamma_multiply(0.8) } else { PANEL };
        painter.rect_filled(r, 5, fill);
        // Чужая точка — серая: появимся на базе.
        let color = if *ours { TEXT } else { Color32::from_gray(110) };
        painter.text(c, Align2::CENTER_CENTER, label, FontId::proportional(13.0), color);
    }
}

/// Прицел: штрихи стоят на краю разброса (`gap` — его радиус в пикселях), так что
/// видно, куда лягут пули. Во время перезарядки по кругу бежит дуга.
pub fn crosshair(painter: &Painter, at: Pos2, gap: f32, reloading: bool, time: f32) {
    let shadow = Stroke::new(3.0, Color32::from_black_alpha(90));
    let stroke = Stroke::new(1.5, Color32::from_white_alpha(230));
    let len = 7.0;
    for dir in [vec2(-1.0, 0.0), vec2(1.0, 0.0), vec2(0.0, -1.0), vec2(0.0, 1.0)] {
        let seg = [at + dir * gap, at + dir * (gap + len)];
        painter.line_segment(seg, shadow);
        painter.line_segment(seg, stroke);
    }
    painter.circle_filled(at, 1.5, Color32::from_white_alpha(230));
    if reloading {
        let r = gap + len + 5.0;
        let start = time * 6.0;
        let points: Vec<Pos2> = (0..=16).map(|i| start + i as f32 / 16.0 * 4.0).map(|a| at + vec2(a.cos(), a.sin()) * r).collect();
        painter.add(egui::Shape::line(points, Stroke::new(2.0, Color32::from_rgb(255, 190, 80))));
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
