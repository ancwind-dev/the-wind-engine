//! Следы боя: подпалины от взрывов и кровь под убитыми. Одна текстура на всю карту
//! (2 байта на тексель: r — подпалина, g — кровь), живёт до смены карты.
//! На GPU уходят только изменённые прямоугольники.

use glam::Vec2;
use std::collections::HashMap;

/// Сколько текселей на метр, если карта не слишком большая.
const TEXELS_PER_METER: f32 = 2.0;
/// Предел стороны текстуры — для огромных карт следы становятся крупнее.
const MAX_SIDE: f32 = 4096.0;

#[derive(Default)]
pub struct Decals {
    width: u32,
    height: u32,
    /// Текселей на метр.
    scale: f32,
    texels: Vec<u8>,
    /// Недавние взрывы, уже оставившие след: (центр, время).
    recent_blasts: Vec<(Vec2, f32)>,
    /// Где у каждого убитого уже есть лужа крови — чтобы не ставить её каждый кадр.
    blood: HashMap<u32, Vec2>,
}

impl Decals {
    /// Новая пустая текстура под карту. Возвращает размер в текселях.
    pub fn reset(&mut self, map_w: u32, map_h: u32) -> (u32, u32) {
        let side = map_w.max(map_h).max(1) as f32;
        self.scale = TEXELS_PER_METER.min(MAX_SIDE / side);
        self.width = ((map_w as f32 * self.scale).ceil() as u32).max(1);
        self.height = ((map_h as f32 * self.scale).ceil() as u32).max(1);
        self.texels = vec![0; (self.width * self.height * 2) as usize];
        self.recent_blasts.clear();
        self.blood.clear();
        (self.width, self.height)
    }

    /// Подпалина под взрывом. Один взрыв приходит много кадров подряд — ставим один раз.
    pub fn blast(&mut self, center: Vec2, radius: f32, time: f32) -> Option<[u32; 4]> {
        self.recent_blasts.retain(|&(_, t)| time - t < 3.0);
        if self.recent_blasts.iter().any(|&(c, _)| c.distance_squared(center) < 0.25) {
            return None;
        }
        self.recent_blasts.push((center, time));
        self.stamp(center, radius * 0.75, 0)
    }

    /// Кровь под убитым. Ставится заново, только если тело оказалось в другом месте.
    pub fn corpse(&mut self, id: u32, pos: Vec2) -> Option<[u32; 4]> {
        if self.blood.get(&id).is_some_and(|&p| p.distance_squared(pos) < 1.0) {
            return None;
        }
        self.blood.insert(id, pos);
        self.stamp(pos, 0.85, 1)
    }

    /// Тексели прямоугольника для отправки на GPU.
    pub fn rect(&self, [x, y, w, h]: [u32; 4]) -> Vec<u8> {
        let mut out = Vec::with_capacity((w * h * 2) as usize);
        for row in y..y + h {
            let start = ((row * self.width + x) * 2) as usize;
            out.extend_from_slice(&self.texels[start..start + (w * 2) as usize]);
        }
        out
    }

    /// Круглое пятно с неровным краем в канал `channel`. Значения только растут —
    /// повторные взрывы не стирают старые следы.
    fn stamp(&mut self, center: Vec2, radius: f32, channel: usize) -> Option<[u32; 4]> {
        if self.texels.is_empty() || radius <= 0.0 {
            return None;
        }
        let c = center * self.scale;
        let r = (radius * self.scale).max(1.0);
        let x0 = ((c.x - r).floor().max(0.0) as u32).min(self.width);
        let y0 = ((c.y - r).floor().max(0.0) as u32).min(self.height);
        let x1 = ((c.x + r).ceil().max(0.0) as u32).min(self.width);
        let y1 = ((c.y + r).ceil().max(0.0) as u32).min(self.height);
        if x0 >= x1 || y0 >= y1 {
            return None;
        }
        // Неровность края зависит от направления — у каждого пятна своя форма.
        let seed = (center.x * 13.7 + center.y * 7.3).fract() * std::f32::consts::TAU;
        for y in y0..y1 {
            for x in x0..x1 {
                let d = Vec2::new(x as f32 + 0.5, y as f32 + 0.5) - c;
                let angle = d.y.atan2(d.x);
                let edge = 1.0 + 0.18 * (angle * 3.0 + seed).sin() + 0.1 * (angle * 7.0 - seed * 2.0).sin();
                let k = 1.0 - (d.length() / (r * edge)).clamp(0.0, 1.0);
                let value = ((k * 2.2).min(1.0) * 255.0) as u8;
                let i = ((y * self.width + x) * 2) as usize + channel;
                self.texels[i] = self.texels[i].max(value);
            }
        }
        Some([x0, y0, x1 - x0, y1 - y0])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blast_stamps_once() {
        let mut d = Decals::default();
        assert_eq!(d.reset(100, 50), (200, 100));
        let rect = d.blast(Vec2::new(10.0, 10.0), 4.0, 0.0).unwrap();
        assert!(d.blast(Vec2::new(10.0, 10.0), 4.0, 0.1).is_none());
        assert_eq!(d.rect(rect).len(), (rect[2] * rect[3] * 2) as usize);
        // Центр пятна закрашен полностью, кровь не тронута.
        let i = ((20 * 200 + 20) * 2) as usize;
        assert_eq!((d.texels[i], d.texels[i + 1]), (255, 0));
    }

    #[test]
    fn edge_of_map_is_clipped() {
        let mut d = Decals::default();
        d.reset(10, 10);
        let [x, y, w, h] = d.corpse(1, Vec2::new(0.1, 9.9)).unwrap();
        assert!(x == 0 && x + w <= 20 && y + h <= 20);
        assert!(d.corpse(1, Vec2::new(0.2, 9.9)).is_none());
    }
}
