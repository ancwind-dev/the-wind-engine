//! Карта — сетка клеток 1×1 м. В каждой клетке хранится прочность стены.
//!
//! Видимость, пули и разрушения работают прямо по этой сетке, без физического движка.

use glam::Vec2;

/// Пустая клетка.
pub const EMPTY: u8 = 0;
/// Неразрушаемая клетка (край карты, скалы).
pub const INDESTRUCTIBLE: u8 = 255;

pub struct Map {
    pub width: u32,
    pub height: u32,
    /// Прочность стены в клетке: 0 — пусто, 255 — неразрушаемо, иначе запас прочности.
    pub solid: Vec<u8>,
    /// Растёт при каждом изменении карты. По нему навигация, отрисовка и
    /// (позже) сеть понимают, что пора обновиться.
    pub revision: u64,
    /// Растёт, только когда клетка стала пустой (стену пробило насквозь).
    /// Поиску пути важно только это, а не каждая царапина.
    pub topology_revision: u64,
}

impl Map {
    pub fn new(width: u32, height: u32) -> Self {
        Map {
            width,
            height,
            solid: vec![EMPTY; (width * height) as usize],
            revision: 0,
            topology_revision: 0,
        }
    }

    #[inline]
    pub fn index(&self, x: i32, y: i32) -> Option<usize> {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            None
        } else {
            Some(y as usize * self.width as usize + x as usize)
        }
    }

    #[inline]
    pub fn cell_of(pos: Vec2) -> (i32, i32) {
        (pos.x.floor() as i32, pos.y.floor() as i32)
    }

    /// За краем карты всё считается сплошным.
    #[inline]
    pub fn is_solid(&self, x: i32, y: i32) -> bool {
        match self.index(x, y) {
            Some(i) => self.solid[i] != EMPTY,
            None => true,
        }
    }

    #[inline]
    pub fn is_solid_at(&self, pos: Vec2) -> bool {
        let (x, y) = Self::cell_of(pos);
        self.is_solid(x, y)
    }

    /// Пересекает ли круг сплошные клетки. Для столкновений солдат со стенами.
    pub fn circle_hits_solid(&self, pos: Vec2, radius: f32) -> bool {
        let (x0, y0) = Self::cell_of(pos - Vec2::splat(radius));
        let (x1, y1) = Self::cell_of(pos + Vec2::splat(radius));
        for y in y0..=y1 {
            for x in x0..=x1 {
                if self.is_solid(x, y) {
                    let nearest = pos.clamp(
                        Vec2::new(x as f32, y as f32),
                        Vec2::new(x as f32 + 1.0, y as f32 + 1.0),
                    );
                    if nearest.distance_squared(pos) < radius * radius {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Ставит стену. Для генерации карты, ревизию не трогает.
    pub fn set(&mut self, x: i32, y: i32, strength: u8) {
        if let Some(i) = self.index(x, y) {
            self.solid[i] = strength;
        }
    }

    /// Идёт по клеткам от `a` к `b` (алгоритм DDA) и возвращает первую сплошную:
    /// долю пути `t` в [0, 1], на которой она встретилась, и её координаты.
    pub fn cast(&self, a: Vec2, b: Vec2) -> Option<(f32, i32, i32)> {
        let d = b - a;
        let (mut cx, mut cy) = Self::cell_of(a);
        let (ex, ey) = Self::cell_of(b);
        let step_x = if d.x > 0.0 { 1 } else { -1 };
        let step_y = if d.y > 0.0 { 1 } else { -1 };
        let t_delta_x = if d.x != 0.0 { (1.0 / d.x).abs() } else { f32::INFINITY };
        let t_delta_y = if d.y != 0.0 { (1.0 / d.y).abs() } else { f32::INFINITY };
        let mut t_max_x = if d.x > 0.0 {
            (cx as f32 + 1.0 - a.x) * t_delta_x
        } else if d.x < 0.0 {
            (a.x - cx as f32) * t_delta_x
        } else {
            f32::INFINITY
        };
        let mut t_max_y = if d.y > 0.0 {
            (cy as f32 + 1.0 - a.y) * t_delta_y
        } else if d.y < 0.0 {
            (a.y - cy as f32) * t_delta_y
        } else {
            f32::INFINITY
        };
        let mut t = 0.0;
        loop {
            if self.is_solid(cx, cy) {
                return Some((t, cx, cy));
            }
            if cx == ex && cy == ey {
                return None;
            }
            if t_max_x < t_max_y {
                t = t_max_x;
                t_max_x += t_delta_x;
                cx += step_x;
            } else {
                t = t_max_y;
                t_max_y += t_delta_y;
                cy += step_y;
            }
            if t > 1.0 {
                return None;
            }
        }
    }

    pub fn line_of_sight(&self, a: Vec2, b: Vec2) -> bool {
        self.cast(a, b).is_none()
    }

    /// Взрыв: снижает прочность разрушаемых стен в круге, к краю урон падает.
    /// Возвращает число разрушенных клеток.
    pub fn damage_circle(&mut self, center: Vec2, radius: f32, damage: f32) -> u32 {
        let (x0, y0) = Self::cell_of(center - Vec2::splat(radius));
        let (x1, y1) = Self::cell_of(center + Vec2::splat(radius));
        let mut destroyed = 0;
        let mut changed = false;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let Some(i) = self.index(x, y) else { continue };
                let hp = self.solid[i];
                if hp == EMPTY || hp == INDESTRUCTIBLE {
                    continue;
                }
                let cell_center = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                let falloff = 1.0 - cell_center.distance(center) / radius;
                if falloff <= 0.0 {
                    continue;
                }
                let hit = (damage * falloff).round().min(254.0) as u8;
                if hit == 0 {
                    continue;
                }
                self.solid[i] = hp.saturating_sub(hit);
                changed = true;
                if self.solid[i] == EMPTY {
                    destroyed += 1;
                }
            }
        }
        if changed {
            self.revision += 1;
        }
        if destroyed > 0 {
            self.topology_revision += 1;
        }
        destroyed
    }
}
