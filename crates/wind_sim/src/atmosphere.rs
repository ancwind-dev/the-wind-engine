//! Атмосфера карты: солнце, цвета света и тени, туман и «плёнка» — насыщенность,
//! контраст, оттенок, обводка, затенение, виньетка. Хранится в `map.ron`.
//!
//! Симуляции она не нужна, но это часть карты: у клиента и сервера она одна и та же.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Atmosphere {
    /// Откуда светит солнце: азимут от севера по часовой, градусы.
    pub sun_azimuth: f32,
    /// Высота солнца над горизонтом, градусы.
    pub sun_elevation: f32,
    /// Цвет солнца (sRGB 0..1) и сила.
    pub sun_color: [f32; 3],
    pub sun_strength: f32,
    /// Цвет неба: им подсвечены поверхности, смотрящие вверх, и им же окрашен туман.
    pub sky_color: [f32; 3],
    /// Цвет тени: им подсвечено всё, куда солнце не попадает.
    pub shadow_color: [f32; 3],
    /// Плотность тумана вдали.
    pub fog: f32,
    /// 0 — плавный свет, 1 — ступенчатый, как в мультфильме.
    pub toon: f32,
    pub saturation: f32,
    pub contrast: f32,
    pub brightness: f32,
    /// Оттенок всей картинки (умножается на цвет).
    pub tint: [f32; 3],
    /// Сила обводки по контурам, 0..1.
    pub outline: f32,
    /// Сила затенения в углах и складках, 0..1.
    pub occlusion: f32,
    /// Затемнение краёв экрана, 0..1.
    pub vignette: f32,
}

impl Default for Atmosphere {
    fn default() -> Self {
        Atmosphere::preset(Preset::Day)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Preset {
    Day,
    Sunset,
    Overcast,
    Desert,
    Night,
}

impl Preset {
    pub const ALL: [Preset; 5] = [Preset::Day, Preset::Sunset, Preset::Overcast, Preset::Desert, Preset::Night];

    pub fn name(self) -> &'static str {
        match self {
            Preset::Day => "День",
            Preset::Sunset => "Закат",
            Preset::Overcast => "Пасмурно",
            Preset::Desert => "Пустыня",
            Preset::Night => "Ночь",
        }
    }
}

impl Atmosphere {
    pub fn preset(p: Preset) -> Self {
        let base = Atmosphere {
            sun_azimuth: 150.0,
            sun_elevation: 55.0,
            sun_color: [1.0, 0.96, 0.88],
            sun_strength: 1.6,
            sky_color: [0.62, 0.70, 0.78],
            shadow_color: [0.36, 0.38, 0.48],
            fog: 0.004,
            toon: 0.7,
            saturation: 0.9,
            contrast: 1.05,
            brightness: 1.0,
            tint: [1.0, 1.0, 1.0],
            outline: 0.7,
            occlusion: 0.5,
            vignette: 0.35,
        };
        match p {
            Preset::Day => base,
            Preset::Sunset => Atmosphere {
                sun_azimuth: 245.0,
                sun_elevation: 22.0,
                sun_color: [1.0, 0.62, 0.36],
                sun_strength: 1.7,
                sky_color: [0.86, 0.56, 0.42],
                shadow_color: [0.38, 0.26, 0.34],
                saturation: 1.0,
                contrast: 1.1,
                tint: [1.0, 0.82, 0.70],
                vignette: 0.5,
                ..base
            },
            Preset::Overcast => Atmosphere {
                sun_elevation: 70.0,
                sun_color: [0.85, 0.88, 0.92],
                sun_strength: 0.9,
                sky_color: [0.66, 0.70, 0.72],
                shadow_color: [0.50, 0.53, 0.56],
                fog: 0.008,
                saturation: 0.7,
                contrast: 0.95,
                tint: [0.86, 0.94, 0.94],
                occlusion: 0.65,
                ..base
            },
            Preset::Desert => Atmosphere {
                sun_elevation: 62.0,
                sun_color: [1.0, 0.93, 0.78],
                sun_strength: 1.8,
                sky_color: [0.86, 0.80, 0.66],
                shadow_color: [0.52, 0.46, 0.44],
                saturation: 0.82,
                tint: [1.0, 0.96, 0.86],
                ..base
            },
            Preset::Night => Atmosphere {
                sun_azimuth: 200.0,
                sun_elevation: 40.0,
                sun_color: [0.55, 0.65, 0.95],
                sun_strength: 0.8,
                sky_color: [0.20, 0.26, 0.40],
                shadow_color: [0.10, 0.12, 0.22],
                fog: 0.006,
                saturation: 0.6,
                contrast: 1.15,
                brightness: 0.9,
                tint: [0.72, 0.82, 1.0],
                vignette: 0.6,
                ..base
            },
        }
    }

    /// Направление НА солнце в 3D (x — восток, y — вверх, z — юг).
    pub fn sun_direction(&self) -> [f32; 3] {
        let (az, el) = (self.sun_azimuth.to_radians(), self.sun_elevation.to_radians());
        // Азимут от севера (−z) по часовой: восток (+x) — 90°.
        [az.sin() * el.cos(), el.sin(), -az.cos() * el.cos()]
    }
}
