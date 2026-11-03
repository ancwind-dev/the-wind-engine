//! Мини-карта в углу и большая карта (клавиша M): стены и покрытие, точки захвата,
//! свои солдаты и замеченные враги (после тумана войны), свой солдат — стрелкой.

use crate::source::View;
use eframe::egui::{self, Align2, Color32, ColorImage, FontId, Painter, Pos2, Rect, Stroke, TextureHandle, TextureOptions, pos2, vec2};
use std::time::Instant;
use wind_sim::Vec2;
use wind_sim::map::{EMPTY, INDESTRUCTIBLE, Map};

const TEAM: [Color32; 2] = [Color32::from_rgb(0x5d, 0xa2, 0xe8), Color32::from_rgb(0xe8, 0x62, 0x58)];

/// Цвета покрытия земли — как в шейдере (`material`), чуть темнее для карты.
const GROUND: [[u8; 3]; 6] = [[196, 156, 70], [196, 156, 70], [110, 94, 70], [172, 158, 116], [72, 72, 74], [146, 144, 136]];

#[derive(Default)]
pub struct Minimap {
    texture: Option<TextureHandle>,
    /// Для какой карты и версии картинка, когда построена.
    key: (u64, u64),
    built: Option<Instant>,
    /// Большая карта по центру экрана.
    pub big: bool,
}

impl Minimap {
    /// Картинка карты: перестраиваем, когда карта изменилась, но не чаще раза в секунду.
    fn texture(&mut self, ctx: &egui::Context, map: &Map, map_key: u64) -> &TextureHandle {
        let key = (map_key, map.revision);
        let stale = self.key != key && self.built.is_none_or(|t| t.elapsed().as_secs_f32() > 1.0);
        if self.texture.is_none() || stale || self.key.0 != map_key {
            let (w, h) = (map.width as usize, map.height as usize);
            let mut pixels = Vec::with_capacity(w * h);
            for i in 0..w * h {
                let c = match map.solid[i] {
                    // Вода: мелко — светлее, глубоко — темнее.
                    EMPTY if map.water.is_some() && water_depth(map, i) > 0.0 => {
                        let k = (water_depth(map, i) / 1.5).min(1.0);
                        let lerp = |a: f32, b: f32| (a + (b - a) * k) as u8;
                        Color32::from_rgb(lerp(110.0, 40.0), lerp(190.0, 105.0), lerp(210.0, 160.0))
                    }
                    EMPTY => {
                        let g = GROUND[(map.ground.get(i).copied().unwrap_or(0) as usize).min(GROUND.len() - 1)];
                        Color32::from_rgb(g[0], g[1], g[2])
                    }
                    INDESTRUCTIBLE => Color32::from_rgb(38, 38, 40),
                    _ => Color32::from_rgb(64, 58, 50),
                };
                pixels.push(c);
            }
            let image = ColorImage { size: [w, h], source_size: egui::vec2(w as f32, h as f32), pixels };
            match &mut self.texture {
                Some(t) => t.set(image, TextureOptions::NEAREST),
                None => self.texture = Some(ctx.load_texture("minimap", image, TextureOptions::NEAREST)),
            }
            self.key = key;
            self.built = Some(Instant::now());
        }
        self.texture.as_ref().unwrap()
    }

    pub fn draw(&mut self, ctx: &egui::Context, painter: &Painter, screen: Rect, (map, map_key): (&Map, u64), view: &View, facing: Option<f32>) {
        let side = if self.big { screen.height() * 0.8 } else { (screen.height() * 0.24).clamp(150.0, 240.0) };
        let aspect = map.width as f32 / map.height.max(1) as f32;
        let size = if aspect >= 1.0 { vec2(side, side / aspect) } else { vec2(side * aspect, side) };
        let rect = if self.big {
            Rect::from_center_size(screen.center(), size)
        } else {
            Rect::from_min_size(screen.right_bottom() - size - vec2(14.0, 14.0), size)
        };
        let texture = self.texture(ctx, map, map_key).id();
        painter.rect_filled(rect.expand(3.0), 4, Color32::from_black_alpha(160));
        painter.image(texture, rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        let to_screen = |p: Vec2| -> Pos2 {
            pos2(rect.left() + p.x / map.width as f32 * rect.width(), rect.top() + p.y / map.height as f32 * rect.height())
        };
        let scale = rect.width() / map.width as f32;

        // Точки захвата — кружок радиуса точки (буква — в конце, поверх солдат).
        for p in &view.points {
            let color = p.owner.map_or(Color32::from_gray(200), |t| TEAM[t as usize & 1]);
            painter.circle(to_screen(p.pos), (p.radius * scale).max(5.0), color.gamma_multiply(0.35), Stroke::new(1.5, color));
        }
        // Солдаты: свои и замеченные враги.
        let dot = if self.big { 3.0 } else { 2.0 };
        for s in &view.soldiers {
            painter.circle_filled(to_screen(s.pos), dot, TEAM[s.team as usize & 1]);
            // Свой отряд — в зелёном кольце.
            if view.me.squad.contains(&s.id) {
                painter.circle_stroke(to_screen(s.pos), dot + 2.0, Stroke::new(1.5, Color32::from_rgb(110, 240, 110)));
            }
        }
        // Точка, которую держит отряд.
        if let Some(at) = view.me.hold {
            painter.circle_stroke(to_screen(at), if self.big { 7.0 } else { 5.0 }, Stroke::new(1.5, Color32::from_rgb(255, 215, 80)));
        }
        // Взрывы — оранжевые вспышки.
        for &(p, radius, age) in &view.blasts {
            painter.circle_stroke(to_screen(p), (radius * scale).max(3.0) * (1.0 + age), Stroke::new(1.5, Color32::from_rgb(255, 170, 60)));
        }
        // Свой солдат — белая стрелка по направлению взгляда.
        if let Some(me) = view.me_pos {
            let c = to_screen(me);
            let a = facing.unwrap_or(0.0);
            let dir = vec2(a.cos(), a.sin());
            let side = vec2(-dir.y, dir.x);
            let len = if self.big { 9.0 } else { 7.0 };
            let tip = c + dir * len;
            let points = vec![tip, c - dir * len * 0.5 + side * len * 0.6, c - dir * len * 0.5 - side * len * 0.6];
            painter.add(egui::Shape::convex_polygon(points, Color32::WHITE, Stroke::new(1.0, Color32::BLACK)));
        }
        // Буквы точек — поверх всего, с тенью: иначе их закрывают точки солдат.
        let font = FontId::proportional(if self.big { 15.0 } else { 11.0 });
        for (i, p) in view.points.iter().enumerate() {
            let c = to_screen(p.pos);
            let letter = (b'A' + i as u8) as char;
            painter.text(c + vec2(1.0, 1.0), Align2::CENTER_CENTER, letter, font.clone(), Color32::BLACK);
            painter.text(c, Align2::CENTER_CENTER, letter, font.clone(), Color32::WHITE);
        }
        if self.big {
            painter.text(rect.center_bottom() + vec2(0.0, 14.0), Align2::CENTER_TOP, "M — закрыть карту", FontId::proportional(13.0), Color32::from_gray(200));
        }
    }
}

/// Глубина воды в середине клетки `i`, м.
fn water_depth(map: &Map, i: usize) -> f32 {
    let w = map.width as usize;
    map.water_depth(wind_sim::Vec2::new((i % w) as f32 + 0.5, (i / w) as f32 + 0.5))
}

