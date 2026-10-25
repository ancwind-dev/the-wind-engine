//! Вьюпорт: вид сверху на симуляцию. Навигация как в Blender:
//! средняя кнопка — двигать, колесо — масштаб, Home — показать всё.

use crate::theme;
use eframe::egui::{
    self, Align2, Color32, ColorImage, FontId, Pos2, Rect, Sense, Stroke, TextureHandle,
    TextureOptions, pos2, vec2,
};
use wind_sim::map::{EMPTY, INDESTRUCTIBLE, Map};
use wind_sim::{ProjectileKind, Vec2, World};

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

/// Вспышка взрыва, живёт меньше секунды.
pub struct Blast {
    pub pos: Vec2,
    pub radius: f32,
    pub age: f32,
}

pub struct Viewport {
    pub camera: Camera,
    pub overlays: Overlays,
    pub follow: bool,
    pub blasts: Vec<Blast>,
    texture: Option<TextureHandle>,
    texture_revision: u64,
    framed: bool,
}

impl Viewport {
    pub fn new() -> Self {
        Viewport {
            camera: Camera { center: Vec2::ZERO, zoom: 1.0 },
            overlays: Overlays { grid: true, bullets: true, flow: false, vision: true },
            follow: false,
            blasts: Vec::new(),
            texture: None,
            texture_revision: u64::MAX,
            framed: false,
        }
    }

    /// Новый матч — заново подгоняем вид и перерисовываем карту.
    pub fn reset(&mut self) {
        self.framed = false;
        self.texture_revision = u64::MAX;
        self.blasts.clear();
    }

    pub fn focus(&mut self, pos: Vec2) {
        self.camera.center = pos;
        self.camera.zoom = self.camera.zoom.max(6.0);
    }

    /// Рисует мир. Возвращает солдата, по которому кликнули (`Some(None)` — клик в пустоту).
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        world: &World,
        selected: Option<u32>,
    ) -> Option<Option<u32>> {
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        let rect = response.rect;
        painter.rect_filled(rect, 0, theme::VIEWPORT);

        if !self.framed {
            self.camera.frame_all(rect, world.map.width as f32);
            self.framed = true;
        }
        self.navigate(ui, &response, rect, world);
        if self.follow
            && let Some(s) = selected.map(|i| &world.soldiers[i as usize])
            && s.alive
        {
            self.camera.center = s.pos;
        }

        self.update_texture(ui.ctx(), &world.map);
        let cam = &self.camera;
        let size = world.map.width as f32;
        let map_rect = Rect::from_two_pos(
            cam.to_screen(rect, Vec2::ZERO),
            cam.to_screen(rect, Vec2::splat(size)),
        );
        if let Some(texture) = &self.texture {
            painter.image(
                texture.id(),
                map_rect,
                Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }

        let visible = Rect::from_two_pos(cam.to_world(rect, rect.min).to_pos(), cam.to_world(rect, rect.max).to_pos())
            .expand(4.0);
        let on_screen = |p: Vec2| visible.contains(p.to_pos());

        if self.overlays.grid && cam.zoom > 1.5 {
            draw_grid(&painter, rect, cam, size);
        }

        // Точки захвата.
        let capture_radius = world.config.capture.radius;
        for (i, point) in world.points.iter().enumerate() {
            let c = cam.to_screen(rect, point.pos);
            let r = capture_radius * cam.zoom;
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
        for (team, base) in world.bases.iter().enumerate() {
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
            && let Some(field) = selected.and_then(|i| world.flow.get(world.soldiers[i as usize].objective as usize))
        {
            let step = if cam.zoom > 8.0 { 2 } else { 4 };
            let (x0, y0) = Map::cell_of(Vec2::new(visible.min.x, visible.min.y));
            let (x1, y1) = Map::cell_of(Vec2::new(visible.max.x, visible.max.y));
            let stroke = Stroke::new(1.0, theme::ACCENT.gamma_multiply(0.8));
            for y in (y0.max(0)..y1.min(world.map.height as i32)).step_by(step) {
                for x in (x0.max(0)..x1.min(world.map.width as i32)).step_by(step) {
                    let at = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                    if let Some(dir) = field.direction(&world.map, at) {
                        let a = cam.to_screen(rect, at);
                        painter.arrow(a, vec2(dir.x, dir.y) * step as f32 * cam.zoom * 0.6, stroke);
                    }
                }
            }
        }

        // Пули и гранаты.
        if self.overlays.bullets {
            let tracer = Stroke::new(1.0, Color32::from_rgba_unmultiplied(0xff, 0xd8, 0x70, 0xc0));
            for p in &world.projectiles {
                if !on_screen(p.pos) {
                    continue;
                }
                match p.kind {
                    ProjectileKind::Bullet => {
                        let tail = p.pos - p.vel * 0.04;
                        painter.line_segment([cam.to_screen(rect, tail), cam.to_screen(rect, p.pos)], tracer);
                    }
                    ProjectileKind::Grenade { fuse, .. } => {
                        let c = cam.to_screen(rect, p.pos);
                        painter.circle_filled(c, (0.4 * cam.zoom).max(2.5), Color32::from_rgb(0x4a, 0x5a, 0x34));
                        let blink = (fuse * 8.0) as i32 % 2 == 0;
                        if blink {
                            painter.circle_stroke(c, (0.4 * cam.zoom).max(2.5) + 1.5, Stroke::new(1.0, theme::ERROR));
                        }
                    }
                }
            }
        }

        // Солдаты. Издалека — точки, вблизи — кружки с направлением взгляда.
        let r = world.config.soldier.radius;
        let close = cam.zoom > 4.0;
        for s in &world.soldiers {
            if !s.alive || !on_screen(s.pos) {
                continue;
            }
            let c = cam.to_screen(rect, s.pos);
            let color = theme::TEAM[s.team as usize];
            if close {
                painter.circle_filled(c, r * cam.zoom, color);
                let look = vec2(s.facing.cos(), s.facing.sin()) * r * cam.zoom * 1.8;
                painter.line_segment([c, c + look], Stroke::new(1.5, color));
            } else {
                let half = (r * cam.zoom).max(1.2);
                painter.rect_filled(Rect::from_center_size(c, vec2(half, half) * 2.0), 0, color);
            }
        }

        // Выделенный солдат: обзор, линия до цели, оранжевая обводка.
        if let Some(s) = selected.map(|i| &world.soldiers[i as usize])
            && s.alive
        {
            let c = cam.to_screen(rect, s.pos);
            if self.overlays.vision {
                painter.circle_stroke(
                    c,
                    world.config.soldier.vision_range * cam.zoom,
                    Stroke::new(1.0, theme::SELECTED.gamma_multiply(0.35)),
                );
                painter.circle_stroke(
                    c,
                    world.config.weapon.range * cam.zoom,
                    Stroke::new(1.0, theme::SELECTED.gamma_multiply(0.6)),
                );
            }
            if let Some(t) = s.target.map(|t| &world.soldiers[t as usize])
                && t.alive
            {
                painter.line_segment([c, cam.to_screen(rect, t.pos)], Stroke::new(1.0, theme::SELECTED));
            }
            painter.circle_stroke(c, (r * cam.zoom).max(3.0) + 2.0, Stroke::new(2.0, theme::SELECTED));
        }

        // Взрывы.
        for b in &self.blasts {
            let t = (b.age / 0.6).min(1.0);
            let c = cam.to_screen(rect, b.pos);
            painter.circle_filled(c, b.radius * cam.zoom * (0.4 + 0.6 * t), Color32::from_rgba_unmultiplied(0xff, 0x9a, 0x3c, ((1.0 - t) * 110.0) as u8));
        }

        draw_header(&painter, rect, cam, world);

        // Клик левой кнопкой — выделить ближайшего солдата.
        if response.clicked_by(egui::PointerButton::Primary)
            && let Some(p) = response.interact_pointer_pos()
        {
            let at = cam.to_world(rect, p);
            let pick = (12.0 / cam.zoom).max(r * 2.0);
            let best = world
                .soldiers
                .iter()
                .enumerate()
                .filter(|(_, s)| s.alive)
                .map(|(i, s)| (i, s.pos.distance(at)))
                .filter(|&(_, d)| d < pick)
                .min_by(|a, b| a.1.total_cmp(&b.1))
                .map(|(i, _)| i as u32);
            return Some(best);
        }
        None
    }

    fn navigate(&mut self, ui: &egui::Ui, response: &egui::Response, rect: Rect, world: &World) {
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
            self.camera.frame_all(rect, world.map.width as f32);
            self.follow = false;
        }
    }

    fn update_texture(&mut self, ctx: &egui::Context, map: &Map) {
        if self.texture.is_some() && self.texture_revision == map.revision {
            return;
        }
        self.texture_revision = map.revision;
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
fn draw_header(painter: &egui::Painter, rect: Rect, cam: &Camera, world: &World) {
    let font = FontId::proportional(12.0);
    let at = rect.min + vec2(12.0, 10.0);
    painter.text(at, Align2::LEFT_TOP, "Вид сверху", font.clone(), theme::TEXT);
    painter.text(
        at + vec2(0.0, 16.0),
        Align2::LEFT_TOP,
        format!("{:.1} пикс/м · карта {} м", cam.zoom, world.map.width),
        font,
        theme::TEXT_DIM,
    );
}

trait ToPos {
    fn to_pos(self) -> Pos2;
}

impl ToPos for Vec2 {
    fn to_pos(self) -> Pos2 {
        pos2(self.x, self.y)
    }
}
