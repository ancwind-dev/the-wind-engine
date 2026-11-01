//! Шаг и бег из записей движений (`data/animations/walk.ron`, `run.ron`, см.
//! `crate::mocap`): клип собирается на модели с её длинами, а поза берётся из круга
//! шага по фазе. Шаг и бег смешиваются по скорости; ноги идут по ходу движения,
//! назад — круг крутится в обратную сторону. Руки с оружием ставит `pose` поверх.

use super::{CharModel, Joints, Pose, two_bone};
use crate::mocap::{Clip, EDGES};
use crate::rwr::VOXEL;
use glam::{Quat, Vec2, Vec3};
use std::f32::consts::{FRAC_PI_2, PI, TAU};
use std::path::Path;

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

    /// Клип круга шага — на этой модели.
    pub(crate) fn gait(&self, clip: &Clip) -> Option<Gait> {
        let j = self.joints?;
        let r = &self.rest;
        let leg = self.leg(j);
        let center = (r[j.hip[0]] + r[j.hip[1]]) * 0.5;
        let foot_y = (r[j.foot[0]].y + r[j.foot[1]].y) * 0.5;
        let at = |name: &str| if name == "hips" { Some(center) } else { j.named(name).map(|i| r[i]) };
        let lengths: Vec<f32> = EDGES.iter().map(|&(a, b)| Some(at(a)?.distance(at(b)?))).collect::<Option<_>>()?;
        let frames = clip
            .frames
            .iter()
            .map(|f| {
                let mut p = r.clone();
                let hips = Vec3::new(center.x + f.hips[0] * leg, foot_y + f.hips[1] * leg, center.z + f.hips[2] * leg);
                let mut place = |name: &str, v: Vec3| {
                    if let Some(i) = j.named(name) {
                        p[i] = v;
                    }
                };
                // Точки — от таза наружу: родитель уже стоит (`EDGES` так упорядочены).
                let mut placed: Vec<(&str, Vec3)> = vec![("hips", hips)];
                for (e, &(from, to)) in EDGES.iter().enumerate() {
                    let base = placed.iter().find(|(n, _)| *n == from).map_or(hips, |x| x.1);
                    let v = base + Vec3::from_array(f.dirs[e]) * lengths[e];
                    placed.push((to, v));
                    place(to, v);
                }
                p
            })
            .collect();
        Some(Gait { frames, stride: clip.stride * leg * VOXEL, peak: clip.peak })
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
    fn recorded_cycles_loop_without_a_jump() {
        let set = set();
        for m in &set.models {
            assert!(m.gaits.is_some() && m.throw.is_some(), "{}: нет клипов из data/animations", m.name);
            for speed in [2.0, 4.5] {
                // Соседние фазы — близкие позы, и на шве круга (2π → 0) тоже.
                let pose = |ph: f32| m.pose(Motion { speed, phase: ph, dir: Vec2::X });
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
            let p = m.pose(Motion { speed: 4.5, phase: k as f32 * TAU / 64.0, dir: Vec2::X });
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

