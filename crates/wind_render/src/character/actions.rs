//! Действия руками: перезарядка и бросок гранаты. Свои, процедурные — как шаг в `pose`:
//! во времени заданы ключевые точки кистей и как держится оружие, локти ставит обратная
//! кинематика. Анимации RWR — чужие, их не берём (см. заметку «Модели и референсы стиля»).

use super::{CharModel, Joints, Pose, side_sign, two_bone};
use crate::rwr::VOXEL;
use glam::{Mat3, Mat4, Quat, Vec3};

/// Как перезаряжается оружие (`reload` в `data/weapons.ron`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ReloadKind {
    /// Отстегнуть магазин, достать новый из подсумка, вставить, передёрнуть затвор.
    #[default]
    Magazine,
    /// Барабан (M32): открыть, по выстрелу в каждое гнездо, закрыть.
    Cylinder,
    /// Трубчатый магазин (ГМ-94): по выстрелу снизу, потом передёрнуть цевьё.
    Pump,
    /// Переломить ствол, вложить выстрел, закрыть.
    Break,
}

impl ReloadKind {
    pub fn from_name(name: &str) -> ReloadKind {
        match name {
            "cylinder" => ReloadKind::Cylinder,
            "pump" => ReloadKind::Pump,
            "break" => ReloadKind::Break,
            _ => ReloadKind::Magazine,
        }
    }
}

/// Что солдат делает руками, кроме стрельбы.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Action {
    /// Перезарядка: `t` — доля от 0 до 1, `rounds` — сколько выстрелов вкладывать
    /// по одному (барабан, трубка).
    Reload { kind: ReloadKind, t: f32, rounds: u32 },
    /// Бросок гранаты: `t` — секунды с начала.
    Throw { t: f32 },
}

/// Сколько длится бросок, с.
pub const THROW_TIME: f32 = 0.75;
/// Когда граната уходит из руки, с от начала броска. Граната в мире вылетает в самом
/// начале (симуляция бросает сразу), поэтому замах короткий.
pub const THROW_RELEASE: f32 = 0.2;

/// Поза с действием.
pub struct Acted {
    pub pose: Pose,
    /// Где оружие — как у `CharModel::weapon_frame`.
    pub weapon: Option<Mat4>,
    /// Граната в руке до броска — точка в вокселях модели.
    pub grenade: Option<Vec3>,
}

/// Как держится оружие: рукоять (у правой кисти), направление ствола и поворот
/// вокруг ствола (плюс — низом к левой руке).
#[derive(Clone, Copy, Debug)]
struct Hold {
    grip: Vec3,
    aim: Vec3,
    roll: f32,
}

impl Hold {
    fn axes(self) -> (Vec3, Vec3) {
        let z = self.aim.normalize_or(Vec3::X);
        let up = (Vec3::Y - z * z.dot(Vec3::Y)).normalize_or(Vec3::Y);
        (z, Quat::from_axis_angle(z, self.roll) * up)
    }

    /// Точка на оружии: `along` вокселей вдоль ствола от рукояти и `below` — вниз.
    fn at(self, along: f32, below: f32) -> Vec3 {
        let (z, y) = self.axes();
        self.grip + z * along - y * below
    }

    fn frame(self) -> Mat4 {
        let (z, y) = self.axes();
        Mat4::from_translation(self.grip * VOXEL) * Mat4::from_mat3(Mat3::from_cols(y.cross(z), y, z))
    }

    fn lerp(self, o: Hold, k: f32) -> Hold {
        Hold { grip: self.grip.lerp(o.grip, k), aim: self.aim.lerp(o.aim, k), roll: self.roll + (o.roll - self.roll) * k }
    }

    /// Ствол — вниз на `down` рад от горизонта и влево (к левой руке) на `left` рад
    /// от направления взгляда; рукоять сдвинута на `shift`.
    fn tilted(self, down: f32, left: f32, roll: f32, shift: Vec3) -> Hold {
        let aim = Quat::from_rotation_y(left) * Quat::from_rotation_z(-down) * Vec3::X;
        Hold { grip: self.grip + shift, aim, roll }
    }
}

fn smooth(u: f32) -> f32 {
    let u = u.clamp(0.0, 1.0);
    u * u * (3.0 - 2.0 * u)
}

/// Доля от 0 до 1 на отрезке `a..b`, плавно.
fn ramp(t: f32, a: f32, b: f32) -> f32 {
    smooth((t - a) / (b - a).max(1e-4))
}

/// Точка по ключам (время, точка): между ключами — плавно.
fn keys(t: f32, k: &[(f32, Vec3)]) -> Vec3 {
    if t <= k[0].0 {
        return k[0].1;
    }
    for w in k.windows(2) {
        if t <= w[1].0 {
            return w[0].1.lerp(w[1].1, ramp(t, w[0].0, w[1].0));
        }
    }
    k[k.len() - 1].1
}

/// Выстрел за выстрелом на отрезке `a..b`: рука ходит к подсумку и вкладывает
/// выстрел в `load`, `rounds` раз.
fn shells(t: f32, a: f32, b: f32, rounds: u32, pouch: Vec3, load: Vec3, push: Vec3) -> Vec3 {
    let n = rounds.clamp(1, 8) as f32;
    let span = ((t - a) / (b - a)).clamp(0.0, 1.0) * n;
    let u = if span >= n { 1.0 } else { span.fract() };
    keys(u, &[(0.0, load), (0.4, pouch), (0.55, pouch), (0.85, load - push * 3.0), (1.0, load)])
}

impl CharModel {
    /// Поза `pose` (из `pose`) с действием руками.
    pub fn act(&self, pose: &Pose, action: Action) -> Acted {
        let Some(j) = self.joints else {
            return Acted { pose: pose.clone(), weapon: self.weapon_frame(pose), grenade: None };
        };
        let mut p = pose.clone();
        // Как оружие держится в обычной позе: от правой кисти к левой.
        let (rh, lh) = (p[j.hand[0]], p[j.hand[1]]);
        let rest = Hold { grip: rh, aim: lh - rh, roll: 0.0 };
        let fore_len = lh.distance(rh);
        // Подсумки — на поясе спереди слева.
        let pouch = p[j.hip[1]] + Vec3::new(5.0, 5.0, -2.5);
        match action {
            Action::Reload { kind, t, rounds } => {
                let t = t.clamp(0.0, 1.0);
                let (hold, left) = reload(kind, t, rounds, rest, fore_len, pouch);
                // Голова смотрит на руки.
                let look = ramp(t, 0.0, 0.15) * (1.0 - ramp(t, 0.85, 1.0));
                p[j.head] += Vec3::new(1.5, -1.2, 0.0) * look;
                let grip = self.reach(&mut p, j, 0, hold.grip, None);
                self.reach(&mut p, j, 1, left, None);
                Acted { pose: p, weapon: Some(Hold { grip, ..hold }.frame()), grenade: None }
            }
            Action::Throw { t } => self.throw(p, j, t, rest, fore_len),
        }
    }

    /// Кисть `side` (0 — правая) тянется к точке `hand`. Возвращает, где кисть оказалась.
    fn reach(&self, p: &mut Pose, j: Joints, side: usize, hand: Vec3, bend: Option<Vec3>) -> Vec3 {
        let r = &self.rest;
        let out = if side == 0 { 1.0 } else { -1.0 };
        let bend = bend.unwrap_or(Vec3::new(0.0, -1.0, 0.6 * out * side_sign(r[j.shoulder[side]].z)));
        let shoulder = p[j.shoulder[side]];
        let upper = r[j.shoulder[side]].distance(r[j.elbow[side]]);
        let fore = r[j.elbow[side]].distance(r[j.hand[side]]);
        p[j.elbow[side]] = two_bone(shoulder, hand, upper, fore, bend);
        p[j.hand[side]] = shoulder + (hand - shoulder).clamp_length_max(upper + fore - 0.01);
        p[j.hand[side]]
    }

    /// Бросок: винтовка уходит вниз в левую руку, корпус закручивается, правая рука
    /// замахивается из-за плеча и бросает через верх, потом всё возвращается.
    fn throw(&self, mut p: Pose, j: Joints, t: f32, rest: Hold, fore_len: f32) -> Acted {
        let t = t.clamp(0.0, THROW_TIME);
        // Поворот плеч вокруг поясницы: минус — правое плечо назад.
        let twist = if t < 0.12 {
            -0.5 * ramp(t, 0.0, 0.12)
        } else if t < 0.3 {
            -0.5 + 0.85 * ramp(t, 0.12, 0.3)
        } else {
            0.35 * (1.0 - ramp(t, 0.3, THROW_TIME))
        };
        let lean = ramp(t, 0.1, 0.25) * (1.0 - ramp(t, 0.3, THROW_TIME));
        let pivot = p[j.mid];
        let turn = Quat::from_rotation_y(twist);
        for i in [j.neck, j.head, j.shoulder[0], j.shoulder[1], j.elbow[0], j.elbow[1], j.hand[0], j.hand[1]] {
            p[i] = pivot + turn * (p[i] - pivot) + Vec3::new(3.0 * lean * (p[i].y - pivot.y) / 12.0, -lean, 0.0);
        }
        // Винтовка — в левой руке, стволом вниз и вперёд, у левого бедра.
        let lowered_left = p[j.hip[1]] + Vec3::new(7.0, 2.0, 1.5);
        let lowered_aim = Vec3::new(0.7, -0.65, -0.25);
        let lowered = Hold { grip: lowered_left - lowered_aim.normalize() * fore_len, aim: lowered_aim, roll: 0.3 };
        let w = ramp(t, 0.0, 0.1) * (1.0 - ramp(t, 0.45, THROW_TIME));
        let hold = rest.lerp(lowered, w);
        self.reach(&mut p, j, 1, hold.at(fore_len, 0.0), None);

        // Правая рука — от плеча (уже повёрнутого).
        let s = p[j.shoulder[0]];
        let right = keys(
            t,
            &[
                (0.0, rest.grip),
                (0.11, s + Vec3::new(-9.0, 7.0, 2.0)),
                (THROW_RELEASE, s + Vec3::new(10.0, 10.0, -1.0)),
                (0.33, s + Vec3::new(13.0, -9.0, -5.0)),
                (0.5, s + Vec3::new(7.0, -12.0, -1.0)),
                (THROW_TIME, hold.grip),
            ],
        );
        // Локоть — в сторону и назад, пока рука бросает.
        let side = ramp(t, 0.0, 0.08) * (1.0 - ramp(t, 0.5, THROW_TIME));
        let bend = Vec3::new(0.0, -1.0, 0.6).lerp(Vec3::new(-0.5, 0.1, 1.0), side);
        let hand = self.reach(&mut p, j, 0, right, Some(bend));
        // Граната в руке — когда рука ушла с оружия и до броска.
        Acted { pose: p, weapon: Some(hold.frame()), grenade: (t > 0.04 && t < THROW_RELEASE).then_some(hand) }
    }
}

/// Перезарядка: как держится оружие и куда тянется левая рука в момент `t` (0..1).
fn reload(kind: ReloadKind, t: f32, rounds: u32, rest: Hold, fore_len: f32, pouch: Vec3) -> (Hold, Vec3) {
    // Оружие наклоняется к груди в начале и возвращается в конце.
    let tilt = ramp(t, 0.0, 0.12) * (1.0 - ramp(t, 0.88, 1.0));
    let posed = match kind {
        // Ствол вниз и влево, низом к левой руке — магазин перед грудью.
        ReloadKind::Magazine => rest.tilted(0.5, 0.45, 0.8, Vec3::new(-2.0, 0.0, -1.5)),
        // Барабан открыт, ствол круто вниз.
        ReloadKind::Cylinder => rest.tilted(0.8, 0.4, 0.5, Vec3::new(-1.0, 0.0, -1.5)),
        // Трубка снизу — к левой руке, оружие у пояса.
        ReloadKind::Pump => rest.tilted(0.25, 0.5, 1.2, Vec3::new(-1.0, -3.0, -1.0)),
        // Переломлен — ствол вниз.
        ReloadKind::Break => rest.tilted(0.9, 0.35, 0.3, Vec3::new(-1.0, 0.0, -1.0)),
    };
    let hold = rest.lerp(posed, tilt);
    let fore = hold.at(fore_len, 0.0);
    let (_, up) = hold.axes();
    let left = match kind {
        ReloadKind::Magazine => {
            let mag = hold.at(5.0, 4.0);
            keys(
                t,
                &[
                    (0.0, fore),
                    (0.14, fore),
                    (0.24, mag),
                    (0.34, mag - up * 8.0),
                    (0.5, pouch),
                    (0.58, pouch),
                    (0.72, mag - up * 6.0),
                    (0.79, mag),
                    // Затвор — над рукоятью: дотянуться и дёрнуть назад.
                    (0.85, hold.at(3.0, -3.0)),
                    (0.9, hold.at(-1.0, -3.0)),
                    (1.0, fore),
                ],
            )
        }
        ReloadKind::Cylinder => {
            let cylinder = hold.at(7.0, 1.5);
            if t < 0.15 {
                keys(t, &[(0.0, fore), (0.15, cylinder)])
            } else if t < 0.8 {
                shells(t, 0.15, 0.8, rounds, pouch, cylinder, up)
            } else {
                // Закрыть барабан — толкнуть вверх, и обратно на цевьё.
                keys(t, &[(0.8, cylinder), (0.86, cylinder + up * 3.0), (1.0, fore)])
            }
        }
        ReloadKind::Pump => {
            let port = hold.at(6.0, 3.5);
            if t < 0.12 {
                keys(t, &[(0.0, fore), (0.12, port)])
            } else if t < 0.76 {
                shells(t, 0.12, 0.76, rounds, pouch, port, up)
            } else {
                // Цевьё — назад и вперёд.
                let (z, _) = hold.axes();
                keys(t, &[(0.76, port), (0.84, fore), (0.9, fore - z * 4.0), (0.95, fore), (1.0, fore)])
            }
        }
        ReloadKind::Break => {
            let breech = hold.at(6.0, 1.0);
            keys(
                t,
                &[
                    (0.0, fore),
                    (0.15, breech),
                    (0.4, pouch),
                    (0.5, pouch),
                    (0.68, breech - up * 3.0),
                    (0.74, breech),
                    // Закрыть: ствол вверх ладонью.
                    (0.84, fore + up * 2.0),
                    (1.0, fore),
                ],
            )
        }
    };
    (hold, left)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::{CharacterSet, Motion};
    use std::path::Path;

    fn set() -> CharacterSet {
        CharacterSet::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")).unwrap()
    }

    const KINDS: [ReloadKind; 4] = [ReloadKind::Magazine, ReloadKind::Cylinder, ReloadKind::Pump, ReloadKind::Break];

    /// Длины рук не меняются, кисти — у плеч на длине руки, не дальше.
    fn arms_intact(m: &CharModel, p: &Pose, what: &str) {
        let j = m.joints.unwrap();
        for side in 0..2 {
            for (a, b) in [(j.shoulder[side], j.elbow[side]), (j.elbow[side], j.hand[side])] {
                let want = m.rest[a].distance(m.rest[b]);
                let got = p[a].distance(p[b]);
                assert!((got - want).abs() < 0.05 * want + 0.05, "{}, {what}: рука растянулась ({got:.1} вместо {want:.1})", m.name);
            }
        }
    }

    #[test]
    fn reloads_start_and_end_in_the_aiming_pose() {
        let set = set();
        for m in &set.models {
            let pose = m.pose(Motion::default());
            let aim = m.weapon_frame(&pose).unwrap();
            for kind in KINDS {
                for t in [0.0, 1.0] {
                    let a = m.act(&pose, Action::Reload { kind, t, rounds: 4 });
                    let diff = (a.weapon.unwrap() - aim).abs().to_cols_array().iter().copied().fold(0.0, f32::max);
                    assert!(diff < 0.02, "{}, {kind:?} при t={t}: оружие не в обычной позе ({diff})", m.name);
                }
                for i in 0..=40 {
                    let a = m.act(&pose, Action::Reload { kind, t: i as f32 / 40.0, rounds: 4 });
                    arms_intact(m, &a.pose, &format!("{kind:?} {i}/40"));
                }
            }
        }
    }

    #[test]
    fn magazine_reload_visits_the_pouch_and_moves_the_weapon() {
        let set = set();
        let m = &set.models[0];
        let j = m.joints.unwrap();
        let pose = m.pose(Motion::default());
        let pouch = pose[j.hip[1]] + Vec3::new(5.0, 5.0, -2.5);
        let at = |t: f32| m.act(&pose, Action::Reload { kind: ReloadKind::Magazine, t, rounds: 30 });
        // Левая рука у подсумка (на поясе слева).
        assert!(at(0.54).pose[j.hand[1]].distance(pouch) < 3.0, "левая рука не дошла до подсумка");
        // Оружие наклонено: ствол ниже, чем в обычной позе.
        let aim = |a: &Acted| a.weapon.unwrap().z_axis.truncate();
        assert!(aim(&at(0.5)).y < aim(&at(0.0)).y - 0.3, "ствол не опустился");
    }

    #[test]
    fn shells_go_in_one_by_one() {
        let set = set();
        let m = &set.models[0];
        let j = m.joints.unwrap();
        let pose = m.pose(Motion::default());
        let pouch = pose[j.hip[1]] + Vec3::new(5.0, 5.0, -2.5);
        // Сколько раз левая рука доходит до подсумка за перезарядку барабана.
        for rounds in [1, 4, 6] {
            let mut visits = 0;
            let mut near = false;
            for i in 0..=600 {
                let a = m.act(&pose, Action::Reload { kind: ReloadKind::Cylinder, t: i as f32 / 600.0, rounds });
                let now = a.pose[j.hand[1]].distance(pouch) < 3.0;
                visits += (now && !near) as u32;
                near = now;
            }
            assert_eq!(visits, rounds, "в барабан на {rounds} — столько же раз к подсумку");
        }
    }

    #[test]
    fn throw_goes_over_the_shoulder_and_returns() {
        let set = set();
        for m in &set.models {
            let j = m.joints.unwrap();
            let pose = m.pose(Motion::default());
            let aim = m.weapon_frame(&pose).unwrap();
            let at = |t: f32| m.act(&pose, Action::Throw { t });
            // Замах: кисть позади и выше плеча; граната в руке.
            let back = at(0.11);
            let (hand, shoulder) = (back.pose[j.hand[0]], back.pose[j.shoulder[0]]);
            assert!(hand.x < shoulder.x && hand.y > shoulder.y, "{}: нет замаха ({hand:?} у плеча {shoulder:?})", m.name);
            assert!(back.grenade.is_some());
            // Бросок: кисть впереди и выше плеча, граната ушла.
            let out = at(THROW_RELEASE + 0.01);
            assert!(out.pose[j.hand[0]].x > out.pose[j.shoulder[0]].x + 5.0, "{}: рука не пошла вперёд", m.name);
            assert!(out.grenade.is_none());
            // Конец — обычная поза с оружием.
            let end = at(THROW_TIME);
            let diff = (end.weapon.unwrap() - aim).abs().to_cols_array().iter().copied().fold(0.0, f32::max);
            assert!(diff < 0.02, "{}: после броска оружие не вернулось ({diff})", m.name);
            for i in 0..=30 {
                arms_intact(m, &at(THROW_TIME * i as f32 / 30.0).pose, &format!("бросок {i}/30"));
            }
        }
    }
}

