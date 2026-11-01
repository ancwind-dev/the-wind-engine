//! Записи движений живых людей (BVH) — в наши клипы.
//!
//! Записи — из базы CMU Graphics Lab Motion Capture Database (mocap.cs.cmu.edu,
//! в BVH её перевёл Bruce Hahne): «бесплатно для исследований и коммерческих
//! проектов», просят указать источник. Сами BVH в проект не кладём — только
//! вырезанные циклы в `data/animations`.
//!
//! Скелет записи (31 сустав) и наш (15 точек) разные, а люди разного роста, поэтому
//! клип хранит не координаты, а **направления отрезков** нашего скелета (бедро → колено,
//! плечо → локоть…) и положение таза в длинах ноги. На модели клип собирается заново
//! с её длинами — любой рост, любые пропорции (см. `Gait`).
//!
//! Оси клипа — наши: x — вперёд (куда идёт), y — вверх, z — вправо.
//!
//! Клип из записи — кадры через равные доли времени. Свой клип (из редактора в студии
//! персонажей) — ключевые кадры в моменты `times`; между ними — плавно.

use glam::{Mat3, Mat4, Quat, Vec3};
use serde::{Deserialize, Serialize};
use std::f32::consts::TAU;

// --- Чтение BVH ----------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq)]
enum Channel {
    Pos(usize),
    Rot(usize),
}

#[derive(Clone, Debug)]
struct Joint {
    name: String,
    parent: Option<usize>,
    offset: Vec3,
    channels: Vec<Channel>,
}

/// Запись BVH: суставы и кадры.
#[derive(Clone, Debug)]
pub struct Bvh {
    joints: Vec<Joint>,
    frames: Vec<Vec<f32>>,
    pub frame_time: f32,
}

impl Bvh {
    pub fn parse(text: &str) -> Result<Bvh, String> {
        let mut t = text.split_whitespace().peekable();
        let mut next = |what: &str| t.next().ok_or_else(|| format!("BVH оборвался: ждали {what}"));
        let num = |s: &str| s.parse::<f32>().map_err(|_| format!("BVH: не число «{s}»"));
        let mut joints: Vec<Joint> = Vec::new();
        let mut stack: Vec<usize> = Vec::new();
        if next("HIERARCHY")? != "HIERARCHY" {
            return Err("BVH: нет HIERARCHY".into());
        }
        loop {
            let word = next("сустав")?;
            match word {
                "ROOT" | "JOINT" | "End" => {
                    let name = match word {
                        "End" => {
                            next("Site")?;
                            format!("{}_End", stack.last().map_or("", |&p| joints[p].name.as_str()))
                        }
                        _ => next("имя")?.to_owned(),
                    };
                    if next("{")? != "{" {
                        return Err(format!("BVH: после {name} нет «{{»"));
                    }
                    joints.push(Joint { name, parent: stack.last().copied(), offset: Vec3::ZERO, channels: Vec::new() });
                    stack.push(joints.len() - 1);
                }
                "OFFSET" => {
                    let j = *stack.last().ok_or("BVH: OFFSET вне сустава")?;
                    joints[j].offset = Vec3::new(num(next("x")?)?, num(next("y")?)?, num(next("z")?)?);
                }
                "CHANNELS" => {
                    let j = *stack.last().ok_or("BVH: CHANNELS вне сустава")?;
                    let n: usize = next("число каналов")?.parse().map_err(|_| "BVH: число каналов")?;
                    for _ in 0..n {
                        let c = next("канал")?;
                        let axis = match &c[..1] {
                            "X" => 0,
                            "Y" => 1,
                            "Z" => 2,
                            _ => return Err(format!("BVH: канал «{c}»")),
                        };
                        joints[j].channels.push(if c.ends_with("position") { Channel::Pos(axis) } else { Channel::Rot(axis) });
                    }
                }
                "}" => {
                    stack.pop();
                }
                "MOTION" => break,
                other => return Err(format!("BVH: неожиданное «{other}»")),
            }
        }
        next("Frames:")?;
        let count: usize = next("число кадров")?.parse().map_err(|_| "BVH: число кадров")?;
        next("Frame")?;
        next("Time:")?;
        let frame_time = num(next("длительность кадра")?)?;
        let width: usize = joints.iter().map(|j| j.channels.len()).sum();
        let mut frames = Vec::with_capacity(count);
        for _ in 0..count {
            let mut f = Vec::with_capacity(width);
            for _ in 0..width {
                f.push(num(next("значение")?)?);
            }
            frames.push(f);
        }
        Ok(Bvh { joints, frames, frame_time })
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    pub fn joint(&self, name: &str) -> Option<usize> {
        self.joints.iter().position(|j| j.name == name)
    }

    /// Положения всех суставов в кадре (единицы и оси записи).
    pub fn positions(&self, frame: usize) -> Vec<Vec3> {
        let values = &self.frames[frame];
        let mut k = 0;
        let mut global: Vec<Mat4> = Vec::with_capacity(self.joints.len());
        for j in &self.joints {
            let mut pos = j.offset;
            let mut rot = Mat3::IDENTITY;
            for &c in &j.channels {
                let v = values[k];
                k += 1;
                match c {
                    Channel::Pos(a) => pos[a] = v,
                    // BVH: повороты в порядке каналов, матрица = Rz·Ry·Rx для «Z Y X».
                    Channel::Rot(a) => {
                        let r = v.to_radians();
                        rot *= match a {
                            0 => Mat3::from_rotation_x(r),
                            1 => Mat3::from_rotation_y(r),
                            _ => Mat3::from_rotation_z(r),
                        };
                    }
                }
            }
            let local = Mat4::from_translation(pos) * Mat4::from_mat3(rot);
            global.push(j.parent.map_or(local, |p| global[p] * local));
        }
        global.iter().map(|m| m.w_axis.truncate()).collect()
    }
}

// --- Наш скелет -----------------------------------------------------------------

/// Отрезки нашего скелета, от таза наружу: (откуда, куда). «hips» — середина между
/// бёдрами (в модели такой точки нет). Порядок — родители раньше детей.
pub const EDGES: [(&str, &str); 15] = [
    ("hips", "righthip"),
    ("hips", "lefthip"),
    ("righthip", "rightknee"),
    ("rightknee", "rightfoot"),
    ("lefthip", "leftknee"),
    ("leftknee", "leftfoot"),
    ("hips", "midspine"),
    ("midspine", "neck"),
    ("neck", "head"),
    ("neck", "rightshoulder"),
    ("neck", "leftshoulder"),
    ("rightshoulder", "rightelbow"),
    ("rightelbow", "righthand"),
    ("leftshoulder", "leftelbow"),
    ("leftelbow", "lefthand"),
];

/// Какой сустав записи CMU соответствует нашей точке. Голова — середина между
/// основанием черепа и макушкой.
const CMU: [(&str, &[&str]); 15] = [
    ("righthip", &["RightUpLeg"]),
    ("lefthip", &["LeftUpLeg"]),
    ("rightknee", &["RightLeg"]),
    ("leftknee", &["LeftLeg"]),
    ("rightfoot", &["RightFoot"]),
    ("leftfoot", &["LeftFoot"]),
    ("midspine", &["Spine"]),
    ("neck", &["Neck1"]),
    ("head", &["Head", "Head_End"]),
    ("rightshoulder", &["RightArm"]),
    ("leftshoulder", &["LeftArm"]),
    ("rightelbow", &["RightForeArm"]),
    ("leftelbow", &["LeftForeArm"]),
    ("righthand", &["RightHand"]),
    ("lefthand", &["LeftHand"]),
];

/// Наши точки в кадре записи (её единицы и оси) по имени; «hips» — середина бёдер.
fn our_points(bvh: &Bvh, frame: usize) -> Result<Vec<(&'static str, Vec3)>, String> {
    let all = bvh.positions(frame);
    let mut out = Vec::with_capacity(16);
    for (name, from) in CMU {
        let mut p = Vec3::ZERO;
        for j in from {
            p += all[bvh.joint(j).ok_or_else(|| format!("в записи нет сустава {j}"))?];
        }
        out.push((name, p / from.len() as f32));
    }
    let hips = (point(&out, "righthip") + point(&out, "lefthip")) * 0.5;
    out.push(("hips", hips));
    Ok(out)
}

fn point(points: &[(&str, Vec3)], name: &str) -> Vec3 {
    points.iter().find(|(n, _)| *n == name).map_or(Vec3::ZERO, |p| p.1)
}

// --- Клип -------------------------------------------------------------------------

/// Кадр клипа: таз и направления отрезков `EDGES`.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct ClipFrame {
    /// Середина бёдер: x, z — отклонение от равномерного хода, y — высота над стопами
    /// (по самой низкой точке стоп за клип); всё — в длинах ноги.
    pub hips: [f32; 3],
    /// Единичные направления отрезков `EDGES`.
    pub dirs: Vec<[f32; 3]>,
}

/// Клип: круг шага (два шага, от выноса левой стопы вперёд до следующего) или
/// действие целиком.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Clip {
    /// Откуда запись — указываем источник, как просит CMU.
    pub source: String,
    /// Круг шага повторяется; действие — нет.
    pub cycle: bool,
    /// Сколько секунд в записи.
    pub duration: f32,
    /// Путь за круг, в длинах ноги (у действия — 0).
    pub stride: f32,
    /// У действия: доля клипа, когда правая кисть быстрее всего (выпуск гранаты).
    #[serde(default)]
    pub peak: f32,
    /// Ключевые кадры: доля клипа (0..1) каждого кадра, по возрастанию. Пусто — кадры
    /// через равные доли (клип из записи).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub times: Vec<f32>,
    pub frames: Vec<ClipFrame>,
}

impl ClipFrame {
    /// Плавно между двумя кадрами: таз — по прямой, направления — по дуге.
    pub fn lerp(&self, other: &ClipFrame, t: f32) -> ClipFrame {
        let hips = Vec3::from_array(self.hips).lerp(Vec3::from_array(other.hips), t).to_array();
        let dirs = self
            .dirs
            .iter()
            .zip(&other.dirs)
            .map(|(&a, &b)| {
                let (a, b) = (Vec3::from_array(a), Vec3::from_array(b));
                (Quat::IDENTITY.slerp(Quat::from_rotation_arc(a, b), t) * a).normalize_or(a).to_array()
            })
            .collect();
        ClipFrame { hips, dirs }
    }
}

/// Сколько кадров в клипе: на глаз хватает, файл маленький.
pub const CLIP_FRAMES: usize = 40;

impl Clip {
    /// Круг шага из записи: самый средний полный круг (от выноса левой стопы вперёд
    /// до следующего), ход — вдоль x, равномерное продвижение убрано.
    pub fn cycle_from(bvh: &Bvh, source: &str) -> Result<Clip, String> {
        // Кадр 0 у записей CMU — поза «Т».
        let frames: Vec<Vec<(&str, Vec3)>> = (1..bvh.frame_count()).map(|f| our_points(bvh, f)).collect::<Result<_, _>>()?;
        if frames.len() < 10 {
            return Err("запись слишком короткая".into());
        }
        let hips: Vec<Vec3> = frames.iter().map(|f| point(f, "hips")).collect();
        let flat = |v: Vec3| Vec3::new(v.x, 0.0, v.z);
        let travel = flat(hips[hips.len() - 1] - hips[0]).normalize_or_zero();
        if travel == Vec3::ZERO {
            return Err("в записи не идут".into());
        }
        // Левая стопа впереди таза больше всего — начало круга.
        let ahead: Vec<f32> = frames.iter().zip(&hips).map(|(f, &h)| (point(f, "leftfoot") - h).dot(travel)).collect();
        // Пик — выраженный (выше середины размаха) и самый высокий в окрестности
        // 0,15 с: иначе дрожание записи даёт ложные пики.
        let (lo, hi) = ahead.iter().fold((f32::MAX, f32::MIN), |(l, h), &v| (l.min(v), h.max(v)));
        let near = (0.15 / bvh.frame_time).round() as usize;
        let peaks: Vec<usize> = (0..ahead.len())
            .filter(|&i| {
                let around = i.saturating_sub(near)..(i + near + 1).min(ahead.len());
                ahead[i] > (lo + hi) * 0.5 && around.clone().all(|j| ahead[j] <= ahead[i]) && i > 0 && i + 1 < ahead.len()
            })
            .collect();
        if peaks.len() < 2 {
            return Err(format!("в записи меньше полного круга шага (выносов левой стопы: {})", peaks.len()));
        }
        let middle = (peaks.len() - 1) / 2;
        let (a, b) = (peaks[middle], peaks[middle + 1]);
        Self::from_range(bvh, &frames, a, b, true, source)
    }

    /// Действие из записи: кадры `from..to` (по счёту записи, кадр 0 — поза «Т»).
    pub fn action_from(bvh: &Bvh, from: usize, to: usize, source: &str) -> Result<Clip, String> {
        let frames: Vec<Vec<(&str, Vec3)>> = (1..bvh.frame_count()).map(|f| our_points(bvh, f)).collect::<Result<_, _>>()?;
        let (a, b) = (from.max(1) - 1, to.min(bvh.frame_count()) - 1);
        if b <= a + 1 {
            return Err("пустой отрезок записи".into());
        }
        Self::from_range(bvh, &frames, a, b, false, source)
    }

    fn from_range(bvh: &Bvh, frames: &[Vec<(&str, Vec3)>], a: usize, b: usize, cycle: bool, source: &str) -> Result<Clip, String> {
        let first = &frames[a];
        let leg = point(first, "righthip").distance(point(first, "rightknee")) + point(first, "rightknee").distance(point(first, "rightfoot"));
        let hips = |f: usize| point(&frames[f], "hips");
        let flat = |v: Vec3| Vec3::new(v.x, 0.0, v.z);
        // Пик действия — когда правая кисть быстрее всего.
        let hand = |f: usize| point(&frames[f], "righthand");
        let peak = (a + 1..b).max_by(|&x, &y| hand(x).distance(hand(x - 1)).total_cmp(&hand(y).distance(hand(y - 1)))).unwrap_or(a);
        // Куда идёт за отрезок; у действия — куда летит кисть в пик (бросок — вперёд,
        // хотя на замахе человек стоит боком).
        let forward = if cycle {
            flat(hips(b) - hips(a)).normalize_or_zero()
        } else {
            flat(hand(peak + 1) - hand(peak - 1)).normalize_or_zero()
        };
        let right = forward.cross(Vec3::Y);
        let ours = |v: Vec3| Vec3::new(v.dot(forward), v.y, v.dot(right));
        let ground = (a..=b).flat_map(|f| [point(&frames[f], "leftfoot").y, point(&frames[f], "rightfoot").y]).fold(f32::MAX, f32::min);
        let stride = if cycle { flat(hips(b) - hips(a)).length() / leg } else { 0.0 };

        let mut out: Vec<ClipFrame> = (0..CLIP_FRAMES)
            .map(|k| {
                // Кадр записи на этой доле отрезка (между соседними — по ближайшему: 120 к/с
                // хватает и так).
                let u = k as f32 / if cycle { CLIP_FRAMES as f32 } else { (CLIP_FRAMES - 1) as f32 };
                let f = (a as f32 + u * (b - a) as f32).round() as usize;
                let p = &frames[f];
                // Таз: без равномерного хода — только покачивание.
                let even = hips(a).lerp(hips(b), u);
                let h = if cycle { hips(f) - even } else { hips(f) - flat(hips(a)) };
                let h = ours(Vec3::new(h.x, 0.0, h.z)) + Vec3::Y * (hips(f).y - ground);
                let dirs = EDGES.iter().map(|&(from, to)| ours(point(p, to) - point(p, from)).normalize_or_zero().to_array()).collect();
                ClipFrame { hips: (h / leg).to_array(), dirs }
            })
            .collect();
        // Шов круга: последний кадр должен переходить в первый — разницу размазываем.
        if cycle {
            let next = &frames[b];
            let end_dirs: Vec<Vec3> = EDGES.iter().map(|&(from, to)| ours(point(next, to) - point(next, from)).normalize_or_zero()).collect();
            let n = out.len() as f32;
            let first_dirs: Vec<Vec3> = out[0].dirs.iter().map(|&d| Vec3::from_array(d)).collect();
            for (k, f) in out.iter_mut().enumerate() {
                let w = k as f32 / n;
                for (e, d) in f.dirs.iter_mut().enumerate() {
                    let fix = (first_dirs[e] - end_dirs[e]) * w;
                    *d = (Vec3::from_array(*d) + fix).normalize_or_zero().to_array();
                }
            }
        }
        let peak = if cycle { 0.0 } else { (peak - a) as f32 / (b - a) as f32 };
        Ok(Clip { source: source.to_owned(), cycle, duration: (b - a) as f32 * bvh.frame_time, stride, peak, times: Vec::new(), frames: out })
    }

    /// Новый клип из двух одинаковых ключей — начало и конец (правится в редакторе).
    pub fn keyed(source: &str, frame: ClipFrame, duration: f32, cycle: bool) -> Clip {
        Clip { source: source.to_owned(), cycle, duration, stride: 0.0, peak: 0.0, times: vec![0.0, 1.0], frames: vec![frame.clone(), frame] }
    }

    /// Ключевые ли кадры (иначе — равномерные из записи).
    pub fn is_keyed(&self) -> bool {
        !self.times.is_empty()
    }

    /// Доля клипа кадра `k`.
    pub fn time_of(&self, k: usize) -> f32 {
        let n = self.frames.len();
        if self.is_keyed() {
            self.times[k]
        } else if self.cycle {
            k as f32 / n as f32
        } else {
            k as f32 / (n - 1).max(1) as f32
        }
    }

    /// Ближайший к доле `u` кадр.
    pub fn nearest(&self, u: f32) -> usize {
        let d = |k: usize| {
            let d = (self.time_of(k) - u).abs();
            if self.cycle { d.min(1.0 - d) } else { d }
        };
        (0..self.frames.len()).min_by(|&a, &b| d(a).total_cmp(&d(b))).unwrap_or(0)
    }

    /// Кадр на доле `u`: между соседними — плавно. Круг — по кругу.
    pub fn sample(&self, u: f32) -> ClipFrame {
        let n = self.frames.len();
        if n == 1 {
            return self.frames[0].clone();
        }
        let u = if self.cycle { u.rem_euclid(1.0) } else { u.clamp(0.0, 1.0) };
        // Соседние кадры: последний перед `u` и следующий (у круга — первый через 1).
        let k = (0..n).rev().find(|&k| self.time_of(k) <= u).unwrap_or(0);
        let (next, t1) = if k + 1 < n {
            (k + 1, self.time_of(k + 1))
        } else if self.cycle {
            (0, self.time_of(0) + 1.0)
        } else {
            return self.frames[k].clone();
        };
        let t0 = self.time_of(k);
        let t = ((u - t0) / (t1 - t0).max(1e-5)).clamp(0.0, 1.0);
        self.frames[k].lerp(&self.frames[next], t)
    }

    /// Те же движения равномерными кадрами — для сборки на модели.
    pub fn uniform(&self) -> Clip {
        if !self.is_keyed() {
            return self.clone();
        }
        let n = CLIP_FRAMES;
        let frames = (0..n).map(|k| self.sample(k as f32 / if self.cycle { n as f32 } else { (n - 1) as f32 })).collect();
        Clip { times: Vec::new(), frames, ..self.clone() }
    }

    /// Ключ на доле `u` (если там уже кадр — он), возвращает номер кадра. У клипа из
    /// записи ключи — все кадры: ближайший.
    pub fn key_at(&mut self, u: f32) -> usize {
        if !self.is_keyed() {
            return self.nearest(u);
        }
        let k = self.nearest(u);
        if (self.time_of(k) - u).abs() < 0.005 {
            return k;
        }
        let frame = self.sample(u);
        let at = self.times.iter().position(|&t| t > u).unwrap_or(self.times.len());
        self.times.insert(at, u);
        self.frames.insert(at, frame);
        at
    }

    /// Убрать ключ (первый и последний не убираются).
    pub fn remove_key(&mut self, k: usize) {
        if self.is_keyed() && k > 0 && k + 1 < self.frames.len() {
            self.times.remove(k);
            self.frames.remove(k);
        }
    }

    /// Заменить кадр `k` на `new`; правка мягко расходится на `soft` соседних кадров
    /// (у клипа из записи кадров много — по одному не поправить).
    pub fn apply(&mut self, k: usize, new: &ClipFrame, soft: usize) {
        let old = self.frames[k].clone();
        let turn: Vec<Quat> = old.dirs.iter().zip(&new.dirs).map(|(&a, &b)| Quat::from_rotation_arc(Vec3::from_array(a), Vec3::from_array(b))).collect();
        let shift = Vec3::from_array(new.hips) - Vec3::from_array(old.hips);
        let n = self.frames.len() as i64;
        let soft = if self.is_keyed() { 0 } else { soft as i64 };
        for d in -soft..=soft {
            let i = k as i64 + d;
            let i = if self.cycle { i.rem_euclid(n) } else if (0..n).contains(&i) { i } else { continue };
            let w = 1.0 - d.abs() as f32 / (soft + 1) as f32;
            let f = &mut self.frames[i as usize];
            for (dir, q) in f.dirs.iter_mut().zip(&turn) {
                *dir = (Quat::IDENTITY.slerp(*q, w) * Vec3::from_array(*dir)).normalize_or_zero().to_array();
            }
            f.hips = (Vec3::from_array(f.hips) + shift * w).to_array();
        }
    }

    /// Клип текстом для `data/animations`: по кадру на строку.
    pub fn to_ron(&self) -> Result<String, String> {
        ron::ser::to_string_pretty(self, ron::ser::PrettyConfig::new().depth_limit(2)).map_err(|e| e.to_string())
    }

    /// Клип из `data/animations`.
    pub fn load(path: &std::path::Path) -> Result<Clip, String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        ron::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Как часто идёт круг шага при скорости `speed` м/с, если нога длиной `leg` м:
    /// радиан фазы на метр пути.
    pub fn stride_rate(&self, leg: f32) -> f32 {
        TAU / (self.stride * leg).max(0.1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Крошечная запись: таз и одна «нога» из двух звеньев, нога качается по Z.
    const TINY: &str = "HIERARCHY
ROOT Hips
{
  OFFSET 0 0 0
  CHANNELS 6 Xposition Yposition Zposition Zrotation Yrotation Xrotation
  JOINT Leg
  {
    OFFSET 0 -10 0
    CHANNELS 3 Zrotation Yrotation Xrotation
    End Site
    {
      OFFSET 0 -10 0
    }
  }
}
MOTION
Frames: 2
Frame Time: 0.5
0 20 0 0 0 0 0 0 0
0 20 5 0 0 0 0 0 90
";

    fn frame(x: f32) -> ClipFrame {
        ClipFrame { hips: [x, 1.0, 0.0], dirs: vec![[1.0, 0.0, 0.0]; EDGES.len()] }
    }

    #[test]
    fn keys_interpolate_and_edits_spread() {
        // Свой клип: два ключа, третий — посередине со сдвигом таза.
        let mut c = Clip::keyed("тест", frame(0.0), 1.0, false);
        let k = c.key_at(0.5);
        assert_eq!((k, c.frames.len()), (1, 3));
        let mut moved = c.frames[1].clone();
        moved.hips[0] = 1.0;
        moved.dirs[0] = [0.0, 1.0, 0.0];
        c.apply(1, &moved, 5);
        // Между ключами — плавно; правка у ключевого клипа на соседей не расходится.
        assert!((c.sample(0.25).hips[0] - 0.5).abs() < 1e-4);
        assert_eq!(c.frames[0], frame(0.0));
        let d = Vec3::from_array(c.sample(0.25).dirs[0]);
        assert!((d.length() - 1.0).abs() < 1e-4 && d.x > 0.6 && d.y > 0.6, "по дуге: {d:?}");
        // В равномерные кадры: начало, середина и конец те же.
        let u = c.uniform();
        assert_eq!(u.frames.len(), CLIP_FRAMES);
        assert!((u.frames[CLIP_FRAMES - 1].hips[0] - 0.0).abs() < 1e-4);
        c.remove_key(1);
        assert_eq!(c.frames.len(), 2);

        // Клип из записи (равномерный круг): правка расходится на соседей, слабея.
        let mut r = Clip { source: String::new(), cycle: true, duration: 1.0, stride: 1.0, peak: 0.0, times: Vec::new(), frames: (0..10).map(|_| frame(0.0)).collect() };
        let mut moved = r.frames[0].clone();
        moved.hips[0] = 1.0;
        r.apply(0, &moved, 2);
        let xs: Vec<f32> = r.frames.iter().map(|f| f.hips[0]).collect();
        assert_eq!(xs[0], 1.0);
        assert!(xs[1] > 0.5 && xs[1] < 1.0 && xs[9] > 0.5, "соседи — слабее, и через шов круга: {xs:?}");
        assert!(xs[3] == 0.0 && xs[5] == 0.0);
    }

    #[test]
    fn reads_bvh_and_bends_joints() {
        let b = Bvh::parse(TINY).unwrap();
        assert_eq!(b.frame_count(), 2);
        assert_eq!(b.frame_time, 0.5);
        let p0 = b.positions(0);
        assert_eq!(p0[b.joint("Leg_End").unwrap()], Vec3::new(0.0, 0.0, 0.0));
        // Поворот сустава на 90° вокруг X: нижнее звено уходит по Z.
        let p1 = b.positions(1);
        let end = p1[b.joint("Leg_End").unwrap()];
        assert!((end - Vec3::new(0.0, 10.0, 5.0 - 10.0)).length() < 1e-3, "{end:?}");
    }
}


