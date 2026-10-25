//! Вьюпорт: вид сверху. Навигация как в Blender:
//! средняя кнопка — двигать, колесо — масштаб, Home — показать всё.
//!
//! Вьюпорт ничего не знает про то, откуда мир: локальная симуляция или сервер.
//! Ему каждый кадр собирают [`Scene`].

use crate::theme;
use eframe::egui::{
    self, Align2, Color32, ColorImage, FontId, Pos2, Rect, Sense, Stroke, TextureHandle,
    TextureOptions, pos2, vec2,
};
use wind_sim::Vec2;
use wind_sim::map::{EMPTY, INDESTRUCTIBLE, Map};
use wind_sim::nav::FlowField;

pub struct SceneSoldier {
    pub id: u32,
    pub pos: Vec2,
    pub facing: f32,
    pub team: u8,
    pub player: bool,
}

pub struct ScenePoint {
    pub pos: Vec2,
    pub owner: Option<u8>,
    pub progress: f32,
}

pub struct SelectedInfo {
    pub pos: Vec2,
    /// Радиусы обзора и стрельбы.
    pub ranges: Option<(f32, f32)>,
    pub target: Option<Vec2>,
}

/// Полоска с состоянием своего солдата при игре по сети.
pub struct Hud {
    pub alive: bool,
    pub health: f32,
    pub ammo: u32,
    pub magazine: u32,
    pub reloading: bool,
    pub grenades: u32,
    pub respawn_in: f32,
    pub kills: u32,
}

pub struct Scene<'a> {
    pub title: String,
    pub map: &'a Map,
    pub bases: [Vec2; 2],
    pub points: Vec<ScenePoint>,
    pub capture_radius: f32,
    pub soldier_radius: f32,
    pub soldiers: Vec<SceneSoldier>,
    /// Пули: (хвост, голова).
    pub bullets: Vec<(Vec2, Vec2)>,
    /// Гранаты: (позиция, сколько до взрыва).
    pub grenades: Vec<(Vec2, f32)>,
    /// Взрывы: (позиция, радиус, возраст в секундах).
    pub blasts: Vec<(Vec2, f32, f32)>,
    pub selected: Option<SelectedInfo>,
    pub flow: Option<&'a FlowField>,
    pub hud: Option<Hud>,
    pub kill_feed: Vec<(String, Color32)>,
}

pub struct ViewportOutput {
    /// Клик по солдату (`Some(None)` — клик в пустоту).
    pub picked: Option<Option<u32>>,
    /// Где курсор в координатах мира.
    pub mouse_world: Option<Vec2>,
    /// Зажата левая кнопка над вьюпортом.
    pub primary_down: bool,
}

pub struct Camera {
    /// Точка мира в центре экрана, м.
    pub center: Vec2,
    /// Пикселей на метр.
    pub zoom: f32,
}

impl Camera {
    fn to_screen(&self, rect: Rect, p: Vec2) -> Pos2 {
        let d = (p - self.center) * self.zoom;
        rect.center() + vec2(d.x, d.y)
    }

    fn to_world(&self, rect: Rect, p: Pos2) -> Vec2 {
        let d = p - rect.center();
        self.center + Vec2::new(d.x, d.y) / self.zoom
    }

    pub fn frame_all(&mut self, rect: Rect, size: f32) {
        self.center = Vec2::splat(size * 0.5);
        self.zoom = (rect.width().min(rect.height()) / size) * 0.95;
    }
}

pub struct Overlays {
    pub grid: bool,
    pub bullets: bool,
    pub flow: bool,
    pub vision: bool,
}

pub struct Viewport {
    pub camera: Camera,
    pub overlays: Overlays,
    /// Камера следует за выделенным солдатом.
    pub follow: bool,
    texture: Option<TextureHandle>,
    /// Карта, нарисованная в текстуре: адрес в памяти и ревизия.
    texture_key: (usize, u64),
    framed: bool,
}

impl Viewport {
    pub fn new() -> Self {
        Viewport {
            camera: Camera { center: Vec2::ZERO, zoom: 1.0 },
            overlays: Overlays { grid: true, bullets: true, flow: false, vision: true },
            follow: false,
            texture: None,
            texture_key: (0, u64::MAX),
            framed: false,
        }
    }

    /// Новый мир — заново подгоняем вид и перерисовываем карту.
    pub fn reset(&mut self) {
        self.framed = false;
        self.texture_key = (0, u64::MAX);
    }

    /// Новый мир, но вид не подгоняем под всю карту: сразу крупно за своим солдатом.
    pub fn reset_close(&mut self, zoom: f32) {
        self.reset();
        self.framed = true;
        self.follow = true;
        self.camera.zoom = zoom;
    }

    pub fn focus(&mut self, pos: Vec2) {
        self.camera.center = pos;
        self.camera.zoom = self.camera.zoom.max(6.0);
    }

    /// `play` — режим игры: левая кнопка стреляет, а не выделяет.
    pub fn show(&mut self, ui: &mut egui::Ui, scene: &Scene, play: bool) -> ViewportOutput {
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = response.rect;
        painter.rect_filled(rect, 0, theme::VIEWPORT);
        let size = scene.map.width as f32;

        if !self.framed {
            self.camera.frame_all(rect, size);
            self.framed = true;
        }
        self.navigate(ui, &response, rect, size);
        if self.follow
            && let Some(s) = &scene.selected
        {
            self.camera.center = s.pos;
        }

        self.update_texture(ui.ctx(), scene.map);
        let cam = &self.camera;
        let map_rect = Rect::from_two_pos(cam.to_screen(rect, Vec2::ZERO), cam.to_screen(rect, Vec2::splat(size)));
        if let Some(texture) = &self.texture {
            painter.image(texture.id(), map_rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }

        let visible = Rect::from_two_pos(cam.to_world(rect, rect.min).to_pos(), cam.to_world(rect, rect.max).to_pos())
            .expand(4.0);
        let on_screen = |p: Vec2| visible.contains(p.to_pos());

        if self.overlays.grid && cam.zoom > 1.5 {
            draw_grid(&painter, rect, cam, size);
        }

        // Точки захвата.
        for (i, point) in scene.points.iter().enumerate() {
            let c = cam.to_screen(rect, point.pos);
            let r = scene.capture_radius * cam.zoom;
            let color = point.owner.map_or(Color32::from_gray(0xb0), |t| theme::TEAM[t as usize]);
            painter.circle_filled(c, r, color.gamma_multiply(0.12));
            painter.circle_stroke(c, r, Stroke::new(1.5, color.gamma_multiply(0.8)));
            let letter = (b'A' + i as u8) as char;
            painter.text(c - vec2(0.0, r + 10.0), Align2::CENTER_CENTER, letter, FontId::proportional(14.0), color);
            // Полоска захвата: влево — синие, вправо — красные.
            let bar = Rect::from_center_size(c - vec2(0.0, r + 22.0), vec2(40.0, 4.0));
            painter.rect_filled(bar, 1, Color32::from_gray(0x20));
            let p = point.progress;
            if p != 0.0 {
                let team = if p < 0.0 { 0 } else { 1 };
                let w = bar.width() * 0.5 * p.abs();
                let fill = if p < 0.0 {
                    Rect::from_min_max(pos2(bar.center().x - w, bar.min.y), pos2(bar.center().x, bar.max.y))
                } else {
                    Rect::from_min_max(pos2(bar.center().x, bar.min.y), pos2(bar.center().x + w, bar.max.y))
                };
                painter.rect_filled(fill, 1, theme::TEAM[team]);
            }
        }

        // Базы.
        for (team, base) in scene.bases.iter().enumerate() {
            let c = cam.to_screen(rect, *base);
            let half = 6.0 * cam.zoom.max(1.0);
            painter.rect_stroke(
                Rect::from_center_size(c, vec2(half, half) * 2.0),
                2,
                Stroke::new(2.0, theme::TEAM[team]),
                egui::StrokeKind::Middle,
            );
        }

        // Поле путей к цели выделенного солдата.
        if self.overlays.flow
            && cam.zoom > 3.0
            && let Some(field) = scene.flow
        {
            let step = if cam.zoom > 8.0 { 2 } else { 4 };
            let (x0, y0) = Map::cell_of(Vec2::new(visible.min.x, visible.min.y));
            let (x1, y1) = Map::cell_of(Vec2::new(visible.max.x, visible.max.y));
            let stroke = Stroke::new(1.0, theme::ACCENT.gamma_multiply(0.8));
            for y in (y0.max(0)..y1.min(scene.map.height as i32)).step_by(step) {
                for x in (x0.max(0)..x1.min(scene.map.width as i32)).step_by(step) {
                    let at = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                    if let Some(dir) = field.direction(scene.map, at) {
                        let a = cam.to_screen(rect, at);
                        painter.arrow(a, vec2(dir.x, dir.y) * step as f32 * cam.zoom * 0.6, stroke);
                    }
                }
            }
        }

        // Пули и гранаты.
        if self.overlays.bullets {
            let tracer = Stroke::new(1.0, Color32::from_rgba_unmultiplied(0xff, 0xd8, 0x70, 0xc0));
            for &(tail, head) in &scene.bullets {
                if on_screen(head) {
                    painter.line_segment([cam.to_screen(rect, tail), cam.to_screen(rect, head)], tracer);
                }
            }
            for &(pos, fuse) in &scene.grenades {
                if !on_screen(pos) {
                    continue;
                }
                let c = cam.to_screen(rect, pos);
                let r = (0.4 * cam.zoom).max(2.5);
                painter.circle_filled(c, r, Color32::from_rgb(0x4a, 0x5a, 0x34));
                if (fuse * 8.0) as i32 % 2 == 0 {
                    painter.circle_stroke(c, r + 1.5, Stroke::new(1.0, theme::ERROR));
                }
            }
        }

        // Солдаты. Издалека — точки, вблизи — кружки с направлением взгляда.
        // Игроки обведены белым.
        let r = scene.soldier_radius;
        let close = cam.zoom > 4.0;
        for s in &scene.soldiers {
            if !on_screen(s.pos) {
                continue;
            }
            let c = cam.to_screen(rect, s.pos);
            let color = theme::TEAM[s.team as usize];
            if close {
                painter.circle_filled(c, r * cam.zoom, color);
                let look = vec2(s.facing.cos(), s.facing.sin()) * r * cam.zoom * 1.8;
                painter.line_segment([c, c + look], Stroke::new(1.5, color));
                if s.player {
                    painter.circle_stroke(c, r * cam.zoom + 1.5, Stroke::new(1.0, theme::TEXT));
                }
            } else {
                let half = (r * cam.zoom).max(if s.player { 2.5 } else { 1.2 });
                painter.rect_filled(Rect::from_center_size(c, vec2(half, half) * 2.0), 0, color);
            }
        }

        // Выделенный солдат: обзор, линия до цели, оранжевая обводка.
        if let Some(sel) = &scene.selected {
            let c = cam.to_screen(rect, sel.pos);
            if self.overlays.vision
                && let Some((vision, range)) = sel.ranges
            {
                painter.circle_stroke(c, vision * cam.zoom, Stroke::new(1.0, theme::SELECTED.gamma_multiply(0.35)));
                painter.circle_stroke(c, range * cam.zoom, Stroke::new(1.0, theme::SELECTED.gamma_multiply(0.6)));
            }
            if let Some(t) = sel.target {
                painter.line_segment([c, cam.to_screen(rect, t)], Stroke::new(1.0, theme::SELECTED));
            }
            painter.circle_stroke(c, (r * cam.zoom).max(3.0) + 2.0, Stroke::new(2.0, theme::SELECTED));
        }

        // Взрывы.
        for &(pos, radius, age) in &scene.blasts {
            let t = (age / 0.6).min(1.0);
            let c = cam.to_screen(rect, pos);
            let alpha = ((1.0 - t) * 110.0) as u8;
            painter.circle_filled(c, radius * cam.zoom * (0.4 + 0.6 * t), Color32::from_rgba_unmultiplied(0xff, 0x9a, 0x3c, alpha));
        }

        draw_header(&painter, rect, cam, scene);
        draw_kill_feed(&painter, rect, &scene.kill_feed);
        if let Some(hud) = &scene.hud {
            draw_hud(&painter, rect, hud);
        }

        let pointer = ui.input(|i| i.pointer.hover_pos()).filter(|p| rect.contains(*p));
        let mouse_world = pointer.map(|p| cam.to_world(rect, p));
        let primary_down = pointer.is_some() && ui.input(|i| i.pointer.primary_down());

        // Клик левой кнопкой — выделить ближайшего солдата.
        let mut picked = None;
        if !play
            && response.clicked_by(egui::PointerButton::Primary)
            && let Some(at) = mouse_world
        {
            let pick = (12.0 / cam.zoom).max(r * 2.0);
            picked = Some(
                scene
                    .soldiers
                    .iter()
                    .map(|s| (s.id, s.pos.distance(at)))
                    .filter(|&(_, d)| d < pick)
                    .min_by(|a, b| a.1.total_cmp(&b.1))
                    .map(|(id, _)| id),
            );
        }
        ViewportOutput { picked, mouse_world, primary_down }
    }

    fn navigate(&mut self, ui: &egui::Ui, response: &egui::Response, rect: Rect, size: f32) {
        if response.dragged_by(egui::PointerButton::Middle) || response.dragged_by(egui::PointerButton::Secondary) {
            let d = response.drag_delta();
            self.camera.center -= Vec2::new(d.x, d.y) / self.camera.zoom;
            self.follow = false;
        }
        if let Some(hover) = response.hover_pos() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y);
            let factor = ui.input(|i| i.zoom_delta()) * (scroll * 0.002).exp();
            if factor != 1.0 {
                // Масштаб к курсору: точка под курсором остаётся на месте.
                let before = self.camera.to_world(rect, hover);
                self.camera.zoom = (self.camera.zoom * factor).clamp(0.2, 60.0);
                let after = self.camera.to_world(rect, hover);
                self.camera.center += before - after;
            }
        }
        if response.hovered() && ui.input(|i| i.key_pressed(egui::Key::Home)) {
            self.camera.frame_all(rect, size);
            self.follow = false;
        }
    }

    fn update_texture(&mut self, ctx: &egui::Context, map: &Map) {
        let key = (map as *const Map as usize, map.revision);
        if self.texture.is_some() && self.texture_key == key {
            return;
        }
        self.texture_key = key;
        let image = map_image(map);
        match &mut self.texture {
            Some(t) => t.set(image, TextureOptions::NEAREST),
            None => self.texture = Some(ctx.load_texture("map", image, TextureOptions::NEAREST)),
        }
    }
}

fn map_image(map: &Map) -> ColorImage {
    let ground = Color32::from_rgb(0x48, 0x4a, 0x46);
    let rock = Color32::from_rgb(0x26, 0x26, 0x26);
    let pixels = map
        .solid
        .iter()
        .map(|&hp| match hp {
            EMPTY => ground,
            INDESTRUCTIBLE => rock,
            // Чем меньше прочность, тем темнее и коричневее стена.
            hp => {
                let t = (hp as f32 / 120.0).min(1.0);
                let lerp = |a: f32, b: f32| (a + (b - a) * t) as u8;
                Color32::from_rgb(lerp(110.0, 172.0), lerp(92.0, 172.0), lerp(70.0, 168.0))
            }
        })
        .collect();
    ColorImage::new([map.width as usize, map.height as usize], pixels)
}

/// Сетка как на полу в Blender: тонкие линии через 10 м, жирнее через 100 м.
fn draw_grid(painter: &egui::Painter, rect: Rect, cam: &Camera, size: f32) {
    let minor = Stroke::new(1.0, Color32::from_white_alpha(10));
    let major = Stroke::new(1.0, Color32::from_white_alpha(26));
    let step = if cam.zoom > 6.0 { 10.0 } else { 50.0 };
    let mut v = 0.0;
    while v <= size {
        let stroke = if (v % 100.0) == 0.0 { major } else { minor };
        let a = cam.to_screen(rect, Vec2::new(v, 0.0));
        let b = cam.to_screen(rect, Vec2::new(v, size));
        painter.line_segment([a, b], stroke);
        let a = cam.to_screen(rect, Vec2::new(0.0, v));
        let b = cam.to_screen(rect, Vec2::new(size, v));
        painter.line_segment([a, b], stroke);
        v += step;
    }
}

/// Надпись в углу вьюпорта, как «User Perspective» в Blender.
fn draw_header(painter: &egui::Painter, rect: Rect, cam: &Camera, scene: &Scene) {
    let font = FontId::proportional(12.0);
    let at = rect.min + vec2(12.0, 10.0);
    painter.text(at, Align2::LEFT_TOP, &scene.title, font.clone(), theme::TEXT);
    painter.text(
        at + vec2(0.0, 16.0),
        Align2::LEFT_TOP,
        format!("{:.1} пикс/м · карта {} м", cam.zoom, scene.map.width),
        font,
        theme::TEXT_DIM,
    );
}

fn draw_kill_feed(painter: &egui::Painter, rect: Rect, feed: &[(String, Color32)]) {
    for (i, (text, color)) in feed.iter().enumerate() {
        let at = rect.right_top() + vec2(-12.0, 10.0 + i as f32 * 16.0);
        painter.text(at, Align2::RIGHT_TOP, text, FontId::proportional(12.0), *color);
    }
}

fn draw_hud(painter: &egui::Painter, rect: Rect, hud: &Hud) {
    let bar = Rect::from_center_size(rect.center_bottom() - vec2(0.0, 30.0), vec2(360.0, 34.0));
    painter.rect_filled(bar, 6, Color32::from_black_alpha(170));
    let font = FontId::proportional(14.0);
    let text = if hud.alive {
        let ammo = if hud.reloading { "перезарядка…".to_string() } else { format!("{} / {}", hud.ammo, hud.magazine) };
        format!(
            "Здоровье {:.0}%   ·   Патроны {ammo}   ·   Гранаты {}   ·   Убийств {}",
            hud.health * 100.0,
            hud.grenades,
            hud.kills
        )
    } else {
        format!("Убит. Появление через {:.1} с", hud.respawn_in)
    };
    let color = if hud.alive { theme::TEXT } else { theme::ERROR };
    painter.text(bar.center(), Align2::CENTER_CENTER, text, font, color);
}

trait ToPos {
    fn to_pos(self) -> Pos2;
}

impl ToPos for Vec2 {
    fn to_pos(self) -> Pos2 {
        pos2(self.x, self.y)
    }
}
