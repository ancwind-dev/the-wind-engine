//! Шаг и бег из записей движений (`data/animations/walk.ron`, `run.ron`, см.
//! `crate::mocap`): клип собирается на модели с её длинами, а поза берётся из круга
//! шага по фазе. Шаг и бег смешиваются по скорости; ноги идут по ходу движения,
//! назад — круг крутится в обратную сторону. Руки с оружием ставит `pose` поверх.

use super::{CharModel, Joints, Pose, two_bone};
use crate::mocap::{Clip, ClipFrame, EDGES};
use crate::rwr::VOXEL;
use glam::{Quat, Vec2, Vec3};
use std::f32::consts::{FRAC_PI_2, PI, TAU};
use std::path::Path;

/// Во сколько раз шаг бега длиннее, чем в записи: бег в игре (6 м/с) быстрее записи
/// (≈ 3 м/с) — иначе ноги семенили бы.
pub const RUN_STRIDE: f32 = 1.3;

/// Клип, собранный на модели: позы по кадрам (воксели, как у `pose`). Круг шага
/// или действие (бросок).
#[derive(Clone, Debug, Default)]
pub struct Gait {
    frames: Vec<Pose>,
    /// Путь за круг (два шага), м; у действия — 0.
    pub stride: f32,
    /// У действия: доля клипа, когда граната уходит из руки (см. `Clip::peak`).
    pub peak: f32,
}

/// Шаг и бег модели.
#[derive(Clone, Debug, Default)]
pub struct Gaits {
    pub walk: Gait,
    pub run: Gait,
}

/// Клипы шага и бега из `data/animations`. Нет файлов — `None`: шаг процедурный.
pub fn load_clips(data_dir: &Path) -> Result<Option<(Clip, Clip)>, String> {
    let dir = data_dir.join("animations");
    let (walk, run) = (dir.join("walk.ron"), dir.join("run.ron"));
    if !walk.exists() || !run.exists() {
        return Ok(None);
    }
    Ok(Some((Clip::load(&walk)?, Clip::load(&run)?)))
}

/// Клип действия `data/animations/<name>.ron`, если есть.
pub fn load_action(data_dir: &Path, name: &str) -> Result<Option<Clip>, String> {
    let path = data_dir.join("animations").join(format!("{name}.ron"));
    if !path.exists() {
        return Ok(None);
    }
    Clip::load(&path).map(Some)
}

impl Joints {
    /// Номер точки по имени, как в `mocap::EDGES`.
    fn named(&self, name: &str) -> Option<usize> {
        Some(match name {
            "head" => self.head,
            "neck" => self.neck,
            "midspine" => self.mid,
            "rightshoulder" => self.shoulder[0],
            "leftshoulder" => self.shoulder[1],
            "rightelbow" => self.elbow[0],
            "leftelbow" => self.elbow[1],
            "righthand" => self.hand[0],
            "lefthand" => self.hand[1],
            "righthip" => self.hip[0],
            "lefthip" => self.hip[1],
            "rightknee" => self.knee[0],
            "leftknee" => self.knee[1],
            "rightfoot" => self.foot[0],
            "leftfoot" => self.foot[1],
            _ => return None,
        })
    }
}

impl CharModel {
    /// Длина ноги в позе покоя, воксели (бедро → колено → стопа, в среднем по двум).
    fn leg(&self, j: Joints) -> f32 {
        let r = &self.rest;
        (0..2).map(|s| r[j.hip[s]].distance(r[j.knee[s]]) + r[j.knee[s]].distance(r[j.foot[s]])).sum::<f32>() / 2.0
    }

    /// Клип — на этой модели (клип с ключами — сначала в равномерные кадры).
    pub(crate) fn gait(&self, clip: &Clip) -> Option<Gait> {
        let sk = self.skeleton()?;
        let clip = clip.uniform();
        let frames = clip.frames.iter().map(|f| sk.pose(f)).collect();
        Some(Gait { frames, stride: clip.stride * sk.leg * VOXEL, peak: clip.peak })
    }

    /// Скелет модели для клипов: длины её отрезков, где таз и стопы.
    pub fn skeleton(&self) -> Option<Skeleton> {
        let j = self.joints?;
        let r = &self.rest;
        let center = (r[j.hip[0]] + r[j.hip[1]]) * 0.5;
        let at = |name: &str| if name == "hips" { Some(center) } else { j.named(name).map(|i| r[i]) };
        let lengths = EDGES.iter().map(|&(a, b)| Some(at(a)?.distance(at(b)?))).collect::<Option<_>>()?;
        let index = EDGES.iter().map(|&(_, b)| j.named(b)).collect::<Option<_>>()?;
        Some(Skeleton {
            lengths,
            index,
            center,
            foot_y: (r[j.foot[0]].y + r[j.foot[1]].y) * 0.5,
            leg: self.leg(j),
            rest: r.clone(),
        })
    }

    /// Ноги и корпус из кругов шага и бега: `None`, если клипов нет. Руки — как в клипе
    /// (их потом ставит `pose`).
    pub(crate) fn gait_pose(&self, speed: f32, phase: f32, dir: Vec2) -> Option<Pose> {
        let g = self.gaits.as_ref()?;
        let j = self.joints?;
        // Назад — круг в обратную сторону, ноги разворачиваем не больше чем на 90°.
        let a = dir.y.atan2(dir.x);
        let (u, turn) = if a.abs() > FRAC_PI_2 + 0.2 {
            (1.0 - (phase / TAU).rem_euclid(1.0), a - PI * a.signum())
        } else {
            ((phase / TAU).rem_euclid(1.0), a.clamp(-FRAC_PI_2, FRAC_PI_2))
        };
        let run = super::run_share(speed);
        let moving = (speed / 0.8).clamp(0.0, 1.0);
        // Шаг и бег на этой доле круга, смешанные по скорости, — сразу в позу, без
        // промежуточных списков: позу считаем каждому солдату в каждом кадре.
        let ((wa, wb, wt), (ra, rb, rt)) = (g.walk.around(u), g.run.around(u));
        let mut p = self.rest.clone();
        for (i, v) in p.iter_mut().enumerate() {
            let w = wa[i].lerp(wb[i], wt);
            let r = ra[i].lerp(rb[i], rt);
            *v = v.lerp(w.lerp(r, run), moving);
        }
        // Бегом шаг длиннее записи: стопы дальше от таза по ходу, колени — заново.
        let long = 1.0 + (RUN_STRIDE - 1.0) * run * moving;
        if long > 1.001 {
            let r = &self.rest;
            for side in 0..2 {
                let hip = p[j.hip[side]];
                let off = p[j.foot[side]] - hip;
                let thigh = r[j.hip[side]].distance(r[j.knee[side]]);
                let shin = r[j.knee[side]].distance(r[j.foot[side]]);
                let foot = hip + Vec3::new(off.x * long, off.y, off.z).clamp_length_max(thigh + shin - 0.05);
                let bend = p[j.knee[side]] - (hip + p[j.foot[side]]) * 0.5;
                p[j.foot[side]] = foot;
                p[j.knee[side]] = two_bone(hip, foot, thigh, shin, bend);
            }
        }
        // Ноги — по ходу движения: стопы поворачиваем вокруг таза, а колени ставим
        // заново, сгибом вперёд — иначе вбок колено гнулось бы в сторону.
        if turn.abs() > 1e-3 {
            let center = (p[j.hip[0]] + p[j.hip[1]]) * 0.5;
            let q = Quat::from_rotation_y(-turn);
            let r = &self.rest;
            for side in 0..2 {
                let foot = center + q * (p[j.foot[side]] - center);
                let thigh = r[j.hip[side]].distance(r[j.knee[side]]);
                let shin = r[j.knee[side]].distance(r[j.foot[side]]);
                p[j.foot[side]] = foot;
                p[j.knee[side]] = two_bone(p[j.hip[side]], foot, thigh, shin, Vec3::X);
            }
        }
        Some(p)
    }
}

/// Скелет модели для клипов: длины отрезков `EDGES`, где таз и стопы в позе покоя.
/// Собирает позу из кадра клипа, кадр — из позы и правит кадр, когда в редакторе
/// тянут точку.
#[derive(Clone, Debug)]
pub struct Skeleton {
    lengths: Vec<f32>,
    /// Номер точки модели для конца каждого отрезка `EDGES`.
    index: Vec<usize>,
    center: Vec3,
    foot_y: f32,
    /// Длина ноги, воксели.
    pub leg: f32,
    rest: Pose,
}

/// Отрезки, которые тянутся вдвоём, как рука или нога: (верхний, нижний) — по номерам
/// `EDGES`. Тянут кисть или стопу — сгиб (локоть, колено) ставится сам.
const CHAINS: [(usize, usize); 4] = [(11, 12), (13, 14), (2, 3), (4, 5)];

impl Skeleton {
    /// Точки кадра: 0 — таз, `1 + e` — конец отрезка `e` (воксели, оси модели).
    pub fn points(&self, f: &ClipFrame) -> Vec<Vec3> {
        let hips = Vec3::new(self.center.x + f.hips[0] * self.leg, self.foot_y + f.hips[1] * self.leg, self.center.z + f.hips[2] * self.leg);
        let mut out = Vec::with_capacity(EDGES.len() + 1);
        out.push(hips);
        for (e, &(from, _)) in EDGES.iter().enumerate() {
            let base = out[Self::point_of(from)];
            out.push(base + Vec3::from_array(f.dirs[e]) * self.lengths[e]);
        }
        out
    }

    /// Номер точки (как в `points`) по имени.
    pub fn point_of(name: &str) -> usize {
        if name == "hips" { 0 } else { 1 + EDGES.iter().position(|&(_, to)| to == name).unwrap_or(0) }
    }

    /// Поза модели из кадра клипа.
    pub fn pose(&self, f: &ClipFrame) -> Pose {
        let mut p = self.rest.clone();
        for (e, v) in self.points(f).into_iter().skip(1).enumerate() {
            p[self.index[e]] = v;
        }
        p
    }

    /// Кадр клипа из позы модели (обратно к `pose`).
    pub fn frame_of(&self, p: &Pose) -> ClipFrame {
        let at = |name: &str| if name == "hips" { (p[self.index[0]] + p[self.index[1]]) * 0.5 } else { p[self.index[Self::point_of(name) - 1]] };
        let hips = at("hips");
        let h = Vec3::new(hips.x - self.center.x, hips.y - self.foot_y, hips.z - self.center.z) / self.leg;
        let dirs = EDGES.iter().map(|&(a, b)| (at(b) - at(a)).normalize_or(Vec3::Y).to_array()).collect();
        ClipFrame { hips: h.to_array(), dirs }
    }

    /// Кадр, в котором точку `point` (номер как в `points`) потянули в `target`. Таз
    /// сдвигается целиком; кисть и стопа тянут за собой локоть и колено (сгиб — в ту же
    /// сторону, что был); остальные — поворачивают свой отрезок.
    pub fn drag(&self, f: &ClipFrame, point: usize, target: Vec3) -> ClipFrame {
        let pts = self.points(f);
        let mut out = f.clone();
        if point == 0 {
            let d = (target - pts[0]) / self.leg;
            out.hips = (Vec3::from_array(f.hips) + d).to_array();
            return out;
        }
        let e = point - 1;
        let start = |e: usize| pts[Self::point_of(EDGES[e].0)];
        if let Some(&(upper, lower)) = CHAINS.iter().find(|c| c.1 == e) {
            let root = start(upper);
            let (a, b) = (self.lengths[upper], self.lengths[lower]);
            let mid = pts[1 + upper];
            let bend = mid - (root + pts[1 + lower]) * 0.5;
            let target = root + (target - root).clamp_length_max(a + b - 0.01);
            let joint = two_bone(root, target, a, b, bend);
            out.dirs[upper] = (joint - root).normalize_or(Vec3::Y).to_array();
            out.dirs[lower] = (target - joint).normalize_or(Vec3::Y).to_array();
        } else {
            out.dirs[e] = (target - start(e)).normalize_or(Vec3::from_array(f.dirs[e])).to_array();
        }
        out
    }
}

impl Gait {
    /// Поза действия на доле `u` (0..1, без повтора).
    pub(crate) fn at_clamped(&self, u: f32) -> Pose {
        let n = self.frames.len();
        let x = u.clamp(0.0, 1.0) * (n - 1) as f32;
        let k = (x.floor() as usize).min(n - 2);
        let t = x - k as f32;
        self.frames[k].iter().zip(&self.frames[k + 1]).map(|(&a, &b)| a.lerp(b, t)).collect()
    }

    /// Соседние кадры круга на доле `u` (0..1) и доля между ними.
    fn around(&self, u: f32) -> (&Pose, &Pose, f32) {
        let n = self.frames.len();
        let x = u.rem_euclid(1.0) * n as f32;
        let k = x.floor() as usize % n;
        (&self.frames[k], &self.frames[(k + 1) % n], x.fract())
    }
}

#[cfg(test)]
mod tests {
    use crate::character::{CharacterSet, Motion};
    use glam::Vec2;
    use std::f32::consts::TAU;
    use std::path::Path;

    fn set() -> CharacterSet {
        CharacterSet::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")).unwrap()
    }

    #[test]
    fn skeleton_round_trips_and_drags_a_hand() {
        let set = set();
        let m = &set.models[0];
        let sk = m.skeleton().unwrap();
        let j = m.joints.unwrap();
        // Поза → кадр → поза: те же точки.
        let pose = m.pose(Motion { speed: 6.0, phase: 1.0, dir: Vec2::X, carry: 0.0 });
        let back = sk.pose(&sk.frame_of(&pose));
        for i in [j.hand[0], j.foot[1], j.head, j.knee[0]] {
            assert!(back[i].distance(pose[i]) < 0.05, "точка {i}: {:?} → {:?}", pose[i], back[i]);
        }
        // Тянем правую кисть вперёд: кисть приходит, локоть сам, длины руки те же.
        let f = sk.frame_of(&pose);
        let hand = super::Skeleton::point_of("righthand");
        let target = sk.points(&f)[super::Skeleton::point_of("rightshoulder")] + glam::Vec3::new(10.0, 2.0, 0.0);
        let moved = sk.pose(&sk.drag(&f, hand, target));
        assert!(moved[j.hand[0]].distance(target) < 0.1, "кисть не дотянулась");
        let upper = m.rest[j.shoulder[0]].distance(m.rest[j.elbow[0]]);
        assert!((moved[j.shoulder[0]].distance(moved[j.elbow[0]]) - upper).abs() < 0.05);
        // Остальное не сдвинулось.
        assert!(moved[j.hand[1]].distance(pose[j.hand[1]]) < 0.05 && moved[j.foot[0]].distance(pose[j.foot[0]]) < 0.05);
    }

    #[test]
    fn recorded_cycles_loop_without_a_jump() {
        let set = set();
        for m in &set.models {
            assert!(m.gaits.is_some() && m.throw.is_some(), "{}: нет клипов из data/animations", m.name);
            for speed in [2.2, 6.0] {
                // Соседние фазы — близкие позы, и на шве круга (2π → 0) тоже.
                let pose = |ph: f32| m.pose(Motion { speed, phase: ph, dir: Vec2::X, carry: 0.0 });
                let step = TAU / 200.0;
                let worst = (0..200)
                    .map(|k| {
                        let (a, b) = (pose(k as f32 * step), pose((k + 1) as f32 * step));
                        a.iter().zip(&b).map(|(x, y)| x.distance(*y)).fold(0.0, f32::max)
                    })
                    .fold(0.0, f32::max);
                assert!(worst < 2.5, "{}, {speed} м/с: рывок {worst:.2} вокселя между соседними фазами", m.name);
            }
        }
    }

    #[test]
    fn recorded_run_keeps_feet_on_the_ground_and_legs_whole() {
        let set = set();
        let m = &set.models[0];
        let j = m.joints.unwrap();
        let (mut low, mut high) = (f32::MAX, f32::MIN);
        for k in 0..64 {
            let p = m.pose(Motion { speed: 6.0, phase: k as f32 * TAU / 64.0, dir: Vec2::X, carry: 0.0 });
            for s in 0..2 {
                low = low.min(p[j.foot[s]].y);
                high = high.max(p[j.foot[s]].y);
                let thigh = m.rest[j.hip[s]].distance(m.rest[j.knee[s]]);
                assert!((p[j.hip[s]].distance(p[j.knee[s]]) - thigh).abs() < 0.6, "бедро тянется");
            }
        }
        // Стопа касается земли (точки стоп в позе покоя — на 0,5) и отрывается на бегу.
        assert!((low - 0.5).abs() < 1.5, "нижняя точка стопы {low:.1}");
        assert!(high > 6.0, "на бегу стопа поднимается: {high:.1}");
    }
}

