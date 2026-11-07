//! Лист кадров: поза за позой человечками — сбоку, спереди и сверху — в PNG. Для разбора
//! анимаций (своих и, временно, RWR) без запуска студии: картинку видно сразу, её можно
//! приложить к заметке. Правые руки и ноги — оранжевые, левые — голубые, корпус — белый.

use std::path::Path;
use wind_render::glam::Vec3;
use wind_render::rwr_anim::POINTS;

/// Палки человечка по именам точек; цвет: 0 — корпус, 1 — правое, 2 — левое.
const STICKS: [(&str, &str, u8); 14] = [
    ("head", "neck", 0),
    ("neck", "midspine", 0),
    ("neck", "rightshoulder", 1),
    ("neck", "leftshoulder", 2),
    ("rightshoulder", "rightelbow", 1),
    ("rightelbow", "righthand", 1),
    ("leftshoulder", "leftelbow", 2),
    ("leftelbow", "lefthand", 2),
    ("midspine", "righthip", 1),
    ("midspine", "lefthip", 2),
    ("righthip", "rightknee", 1),
    ("rightknee", "rightfoot", 1),
    ("lefthip", "leftknee", 2),
    ("leftknee", "leftfoot", 2),
];

const COLORS: [[u8; 3]; 3] = [[235, 235, 235], [255, 150, 60], [90, 170, 255]];
const CELL: usize = 150;
/// Пикселей на воксель.
const SCALE: f32 = 2.1;

struct Canvas {
    w: usize,
    h: usize,
    px: Vec<[u8; 3]>,
}

impl Canvas {
    fn new(w: usize, h: usize) -> Self {
        Canvas { w, h, px: vec![[28, 30, 34]; w * h] }
    }

    fn dot(&mut self, x: f32, y: f32, r: f32, c: [u8; 3]) {
        let (x0, x1) = ((x - r).floor() as i64, (x + r).ceil() as i64);
        let (y0, y1) = ((y - r).floor() as i64, (y + r).ceil() as i64);
        for py in y0..=y1 {
            for px in x0..=x1 {
                if px < 0 || py < 0 || px >= self.w as i64 || py >= self.h as i64 {
                    continue;
                }
                let d = ((px as f32 + 0.5 - x).powi(2) + (py as f32 + 0.5 - y).powi(2)).sqrt();
                let a = (r + 0.5 - d).clamp(0.0, 1.0);
                if a > 0.0 {
                    let p = &mut self.px[py as usize * self.w + px as usize];
                    for i in 0..3 {
                        p[i] = (p[i] as f32 * (1.0 - a) + c[i] as f32 * a) as u8;
                    }
                }
            }
        }
    }

    fn line(&mut self, a: (f32, f32), b: (f32, f32), r: f32, c: [u8; 3]) {
        let n = ((b.0 - a.0).hypot(b.1 - a.1) * 2.0).ceil().max(1.0) as usize;
        for i in 0..=n {
            let t = i as f32 / n as f32;
            self.dot(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t, r, c);
        }
    }

    fn rect(&mut self, x: usize, y: usize, w: usize, h: usize, c: [u8; 3]) {
        for py in y..(y + h).min(self.h) {
            for px in x..(x + w).min(self.w) {
                self.px[py * self.w + px] = c;
            }
        }
    }

    fn save(&self, path: &Path) -> Result<(), String> {
        let file = std::fs::File::create(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), self.w as u32, self.h as u32);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(|e| e.to_string())?;
        let flat: Vec<u8> = self.px.iter().flatten().copied().collect();
        writer.write_image_data(&flat).map_err(|e| e.to_string())
    }
}

/// Вид: какие оси точки — в x и y картинки (y картинки растёт вниз).
#[derive(Clone, Copy)]
enum View {
    /// Сбоку, справа от солдата: вперёд — вправо, вверх — вверх.
    Side,
    /// Спереди: правая сторона солдата — слева на картинке.
    Front,
    /// Сверху: вперёд — вверх, правая сторона — вправо.
    Top,
}

fn project(v: View, p: Vec3, origin: Vec3) -> (f32, f32) {
    let d = p - origin;
    let c = CELL as f32;
    match v {
        View::Side => (c * 0.5 + d.x * SCALE, c * 0.95 - d.y * SCALE),
        View::Front => (c * 0.5 - d.z * SCALE, c * 0.95 - d.y * SCALE),
        View::Top => (c * 0.5 + d.z * SCALE, c * 0.5 - d.x * SCALE),
    }
}

/// Лист: столбцы — позы (15 точек в порядке `POINTS`), строки — сбоку, спереди, сверху.
/// Начало каждой позы — точка на земле под серединой бёдер первой позы: видно, как тело
/// опускается и смещается за клип.
pub fn save(poses: &[[Vec3; 15]], path: &Path) -> Result<(), String> {
    let at = |name: &str| POINTS.iter().position(|p| *p == name).expect("имя точки");
    let first = poses.first().ok_or("нет поз")?;
    let hip = (first[at("righthip")] + first[at("lefthip")]) * 0.5;
    let origin = Vec3::new(hip.x, 0.0, hip.z);
    let views = [View::Side, View::Front, View::Top];
    let mut c = Canvas::new(CELL * poses.len(), CELL * views.len());
    for (row, &view) in views.iter().enumerate() {
        for (col, pose) in poses.iter().enumerate() {
            let (ox, oy) = ((col * CELL) as f32, (row * CELL) as f32);
            // Рамка ячейки, земля и деления через 8 вокселей (¼ м).
            c.rect(col * CELL, row * CELL, 1, CELL, [55, 58, 64]);
            c.rect(col * CELL, row * CELL, CELL, 1, [55, 58, 64]);
            if !matches!(view, View::Top) {
                let ground = oy + CELL as f32 * 0.95;
                c.rect(col * CELL + 1, ground as usize, CELL - 1, 1, [80, 84, 92]);
                for k in -8..=8 {
                    let x = ox + CELL as f32 * 0.5 + k as f32 * 8.0 * SCALE;
                    if x > ox && x < ox + CELL as f32 {
                        c.rect(x as usize, ground as usize - 2, 1, 2, [80, 84, 92]);
                    }
                }
            } else {
                c.rect(col * CELL + CELL / 2, row * CELL + 2, 1, CELL - 4, [45, 48, 54]);
                c.rect(col * CELL + 2, row * CELL + CELL / 2, CELL - 4, 1, [45, 48, 54]);
            }
            let p = |name: &str| {
                let (x, y) = project(view, pose[at(name)], origin);
                (ox + x, oy + y)
            };
            // Сначала левая сторона (дальняя на виде сбоку), потом корпус, потом правая.
            for side in [2u8, 0, 1] {
                for &(a, b, s) in STICKS.iter().filter(|s| s.2 == side) {
                    c.line(p(a), p(b), 1.4, COLORS[s as usize]);
                }
            }
            let (hx, hy) = p("head");
            c.dot(hx, hy, 3.2, COLORS[0]);
        }
    }
    c.save(path)
}
