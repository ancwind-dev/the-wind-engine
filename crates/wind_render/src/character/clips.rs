//! Клипы стандарта анимаций (`crate::anim`), собранные на модели: ключи — в направления
//! отрезков скелета (длины рук и ног — свои у каждой модели), между ключами — плавно по
//! дуге, и сразу в равномерные кадры: в игре поза берётся из соседних кадров по прямой —
//! это дёшево на тысячу солдат.

use super::{CharModel, Pose};
use crate::anim::{AnimClip, Body, Event, Hands};
use crate::mocap::ClipFrame;
use crate::rwr::VOXEL;
use glam::Vec3;

/// Кадров на секунду клипа при сборке.
const FPS: f32 = 30.0;

/// Оружие в кадре: рукоять, ствол, поворот вокруг ствола.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeaponAt {
    pub grip: Vec3,
    pub aim: Vec3,
    pub roll: f32,
}

/// Клип на модели.
#[derive(Clone, Debug, Default)]
pub struct Built {
    pub key: String,
    frames: Vec<Pose>,
    weapons: Vec<Option<WeaponAt>>,
    /// Секунд в клипе.
    pub seconds: f32,
    pub looped: bool,
    /// Путь за круг, м (у движения).
    pub stride: f32,
    pub body: Body,
    pub hands: Hands,
    /// Метки: (доля клипа 0..1, метка).
    pub events: Vec<(f32, Event)>,
    /// Повторяемый отрезок, доли клипа.
    pub repeat: Option<(f32, f32)>,
}

impl CharModel {
    /// Поза модели из 15 точек клипа (по именам).
    pub fn pose_from_points(&self, points: &[Vec3; 15]) -> Option<Pose> {
        let j = self.joints?;
        let mut p = self.rest.clone();
        for (i, name) in crate::anim::NAMES.iter().enumerate() {
            p[j.named(name)?] = points[i];
        }
        Some(p)
    }

    /// 15 точек клипа (по именам) из позы модели.
    pub fn points_of(&self, pose: &Pose) -> Option<[Vec3; 15]> {
        let j = self.joints?;
        let mut out = [Vec3::ZERO; 15];
        for (i, name) in crate::anim::NAMES.iter().enumerate() {
            out[i] = pose[j.named(name)?];
        }
        Some(out)
    }

    /// Клип на этой модели. `None` — у модели нет скелета.
    pub fn build_clip(&self, clip: &AnimClip) -> Option<Built> {
        let sk = self.skeleton()?;
        let frames: Vec<ClipFrame> = clip.keys.iter().map(|k| self.pose_from_points(&k.points.to_array()).map(|p| sk.frame_of(&p))).collect::<Option<_>>()?;
        let seconds = clip.seconds();
        let length = clip.length.max(1e-4);
        // Круг — без повтора первого кадра в конце; раз — с последним кадром.
        let n = ((seconds * FPS).ceil() as usize).max(2);
        let count = if clip.looped { n } else { n + 1 };
        let mut poses = Vec::with_capacity(count);
        let mut weapons = Vec::with_capacity(count);
        for i in 0..count {
            let t = length * i as f32 / n as f32;
            let (a, b, u) = clip.around(t);
            poses.push(sk.pose(&frames[a].lerp(&frames[b], u)));
            let w = match (clip.keys[a].weapon, clip.keys[b].weapon) {
                (Some(wa), Some(wb)) => Some(WeaponAt {
                    grip: Vec3::from_array(wa.grip).lerp(Vec3::from_array(wb.grip), u),
                    aim: Vec3::from_array(wa.aim).lerp(Vec3::from_array(wb.aim), u).normalize_or(Vec3::X),
                    roll: wa.roll + (wb.roll - wa.roll) * u,
                }),
                (Some(w), None) | (None, Some(w)) => Some(WeaponAt { grip: Vec3::from_array(w.grip), aim: Vec3::from_array(w.aim).normalize_or(Vec3::X), roll: w.roll }),
                (None, None) => None,
            };
            weapons.push(w);
        }
        Some(Built {
            key: clip.key.clone(),
            frames: poses,
            weapons,
            seconds,
            looped: clip.looped,
            stride: clip.stride * sk.leg * VOXEL,
            body: clip.body,
            hands: clip.hands,
            events: clip.events().into_iter().map(|(t, e)| (t / length, e)).collect(),
            repeat: clip.repeat.map(|(a, b)| (a / length, b / length)),
        })
    }
}

impl Built {
    /// Кадр на доле `u` (круг — по кругу, раз — от 0 до 1) и доля до следующего.
    fn index(&self, u: f32) -> (usize, usize, f32) {
        let n = self.frames.len();
        if n == 1 {
            return (0, 0, 0.0);
        }
        if self.looped {
            let x = u.rem_euclid(1.0) * n as f32;
            let k = (x.floor() as usize) % n;
            (k, (k + 1) % n, x.fract())
        } else {
            let x = u.clamp(0.0, 1.0) * (n - 1) as f32;
            let k = (x.floor() as usize).min(n - 2);
            (k, k + 1, x - k as f32)
        }
    }

    /// Поза на доле клипа `u`.
    pub fn at(&self, u: f32) -> Pose {
        let (a, b, t) = self.index(u);
        self.frames[a].iter().zip(&self.frames[b]).map(|(&x, &y)| x.lerp(y, t)).collect()
    }

    /// Поза на доле `u` — сразу в `out` (без выделения памяти: позу считают каждому
    /// солдату в каждом кадре).
    pub fn at_into(&self, u: f32, out: &mut Pose) {
        let (a, b, t) = self.index(u);
        out.clear();
        out.extend(self.frames[a].iter().zip(&self.frames[b]).map(|(&x, &y)| x.lerp(y, t)));
    }

    /// Соседние кадры на доле `u` и доля между ними — смешивать два клипа без копий.
    pub fn around(&self, u: f32) -> (&Pose, &Pose, f32) {
        let (a, b, t) = self.index(u);
        (&self.frames[a], &self.frames[b], t)
    }

    /// Оружие на доле `u`, если клип его задаёт.
    pub fn weapon_at(&self, u: f32) -> Option<WeaponAt> {
        let (a, b, t) = self.index(u);
        match (self.weapons[a], self.weapons[b]) {
            (Some(wa), Some(wb)) => Some(WeaponAt { grip: wa.grip.lerp(wb.grip, t), aim: wa.aim.lerp(wb.aim, t).normalize_or(Vec3::X), roll: wa.roll + (wb.roll - wa.roll) * t }),
            (w, None) | (None, w) => w,
        }
    }

    /// Держат ли оружие правая и левая руки на доле `u` (по меткам до этого момента).
    pub fn hands_at(&self, u: f32) -> (bool, bool) {
        let (mut strong, mut weak) = (true, true);
        for &(t, e) in &self.events {
            if t > u + 1e-4 {
                break;
            }
            match e {
                Event::StrongHand(v) => strong = v,
                Event::WeakHand(v) => weak = v,
                _ => {}
            }
        }
        (strong, weak)
    }

    /// Когда граната уходит из руки (доля клипа).
    pub fn release(&self) -> Option<f32> {
        self.events.iter().find(|(_, e)| *e == Event::Release).map(|&(t, _)| t)
    }

    /// Доля клипа для действия с `rounds` повторами отрезка `repeat` при общей доле `t`
    /// (0..1 всего действия): начало, отрезок `rounds` раз, конец — в одну шкалу.
    pub fn repeated(&self, t: f32, rounds: u32) -> f32 {
        let t = t.clamp(0.0, 1.0);
        let Some((a, b)) = self.repeat else { return t };
        let n = rounds.clamp(1, 8) as f32;
        let seg = b - a;
        let total = a + seg * n + (1.0 - b);
        let x = t * total;
        if x < a {
            x
        } else if x < a + seg * n {
            a + (x - a) % seg
        } else {
            b + (x - a - seg * n)
        }
    }

    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anim::{Key, Points};

    fn model() -> CharModel {
        let set = crate::character::CharacterSet::load(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")).unwrap();
        set.models.into_iter().next().unwrap()
    }

    #[test]
    fn built_clip_keeps_bone_lengths_between_far_keys() {
        let m = model();
        let rest = m.points_of(&m.rest).unwrap();
        // Рука из «вниз» в «вперёд»: по прямой между ключами кисть подошла бы к плечу,
        // а собранный клип ведёт её по дуге — длина руки та же.
        let mut down = rest;
        let (sh, el, ha) = (2, 4, 6);
        down[el] = rest[sh] + Vec3::new(0.0, -8.0, 0.0);
        down[ha] = rest[sh] + Vec3::new(0.0, -15.0, 0.0);
        let mut fwd = rest;
        fwd[el] = rest[sh] + Vec3::new(8.0, 0.0, 0.0);
        fwd[ha] = rest[sh] + Vec3::new(15.0, 0.0, 0.0);
        let clip = AnimClip {
            key: "рука".into(),
            about: String::new(),
            source: String::new(),
            looped: false,
            length: 1.0,
            speed: 1.0,
            speed_spread: 0.0,
            stride: 0.0,
            body: Body::Full,
            hands: Hands::Clip,
            repeat: None,
            keys: vec![
                Key { time: 0.0, points: Points::from_array(down), weapon: None, events: vec![] },
                Key { time: 1.0, points: Points::from_array(fwd), weapon: None, events: vec![Event::Release] },
            ],
        };
        let b = m.build_clip(&clip).unwrap();
        let j = m.joints.unwrap();
        let reach = |p: &Pose| p[j.hand[0]].distance(p[j.shoulder[0]]);
        let full = reach(&b.at(0.0));
        assert!((reach(&b.at(0.5)) - full).abs() < 0.3, "рука укоротилась посередине: {} из {full}", reach(&b.at(0.5)));
        assert_eq!(b.release(), Some(1.0));
    }

    #[test]
    fn repeated_segment_plays_rounds_times() {
        let b = Built { repeat: Some((0.2, 0.6)), ..Default::default() };
        // Начало и конец — как в клипе, середина — отрезок три раза.
        assert!((b.repeated(0.0, 3) - 0.0).abs() < 1e-5);
        assert!((b.repeated(1.0, 3) - 1.0).abs() < 1e-5);
        let total = 0.2 + 0.4 * 3.0 + 0.4;
        // Посередине второго повтора — середина отрезка.
        assert!((b.repeated((0.2 + 0.4 * 1.5) / total, 3) - 0.4).abs() < 1e-4);
    }
}
