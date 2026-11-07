//! Генератор наших клипов в стандарте анимаций (`wind_lab --make-clips`).
//!
//! - Бросок — из записи движений CMU (старый файл `throw.ron` переводится один раз).
//! - Шаг, бег и ползком — свои кривые по устройству и размахам RWR (см. «Как устроены
//!   анимации RWR»).
//! - Перезарядки — из процедурных действий (`actions.rs`): они и есть «автор» этих клипов.
//! - «Стоит», присед, лечь, лёжа, ползком, встать — позы задаём здесь сами: где таз, как
//!   наклонён корпус, где стопы и кисти; колени и локти ставит обратная кинематика.
//!   Устройство и числа (длительности, высоты, порядок фаз) — по разбору RWR, кадры — свои.
//!
//! Готовые клипы потом правятся в студии персонажей; генератор не перезаписывает то, что
//! уже есть, без `--force`.

use super::{Action, CharModel, Joints, Motion, Pose, ReloadKind, THROW_RELEASE, THROW_TIME, two_bone};
use crate::anim::{AnimClip, Body, Event, Hands, Key, Points, Side, WeaponKey};
use crate::mocap::Clip;
use crate::rwr::VOXEL;
use glam::{Mat3, Mat4, Vec3};
use std::f32::consts::{PI, TAU};
use std::path::Path;

/// Источник своих поз — для поля `source`.
const OWN: &str = "свой: позы задаёт генератор wind_lab --make-clips (character/make.rs)";

/// Скорость ползком, м/с, и приседом — по ней длина круга (чтобы стопы не скользили).
pub const PRONE_SPEED: f32 = 0.75;
pub const CROUCH_SPEED: f32 = 1.85;

/// Поза, заданная крупно: таз, наклон и поворот корпуса, стопы и кисти.
#[derive(Clone, Copy)]
struct Shape {
    /// Середина бёдер, воксели.
    hips: Vec3,
    /// Наклон корпуса вперёд, рад (0 — прямо, π/2 — лёжа).
    pitch: f32,
    /// Поворот плеч вокруг вертикали, рад (плюс — левое плечо вперёд).
    yaw: f32,
    /// Голова поднята относительно корпуса, рад.
    head_up: f32,
    feet: [Vec3; 2],
    /// Куда гнутся колени.
    knees: [Vec3; 2],
    hands: [Vec3; 2],
    /// Куда гнутся локти.
    elbows: [Vec3; 2],
}

impl CharModel {
    fn shaped(&self, j: Joints, s: &Shape) -> Pose {
        let r = &self.rest;
        let c0 = (r[j.hip[0]] + r[j.hip[1]]) * 0.5;
        let turn = Mat3::from_rotation_y(s.yaw) * Mat3::from_rotation_z(-s.pitch);
        let mut p = r.clone();
        for i in [j.mid, j.neck, j.shoulder[0], j.shoulder[1]] {
            p[i] = s.hips + turn * (r[i] - c0);
        }
        let head = Mat3::from_rotation_y(s.yaw) * Mat3::from_rotation_z(-(s.pitch - s.head_up));
        p[j.head] = p[j.neck] + head * (r[j.head] - r[j.neck]);
        let pelvis = Mat3::from_rotation_y(s.yaw * 0.3);
        for side in 0..2 {
            p[j.hip[side]] = s.hips + pelvis * (r[j.hip[side]] - c0);
            let (thigh, shin) = (r[j.hip[side]].distance(r[j.knee[side]]), r[j.knee[side]].distance(r[j.foot[side]]));
            let hip = p[j.hip[side]];
            let foot = hip + (s.feet[side] - hip).clamp_length_max(thigh + shin - 0.05);
            p[j.knee[side]] = two_bone(hip, foot, thigh, shin, s.knees[side]);
            p[j.foot[side]] = foot;
            let (upper, fore) = (r[j.shoulder[side]].distance(r[j.elbow[side]]), r[j.elbow[side]].distance(r[j.hand[side]]));
            let sh = p[j.shoulder[side]];
            let hand = sh + (s.hands[side] - sh).clamp_length_max(upper + fore - 0.05);
            p[j.elbow[side]] = two_bone(sh, hand, upper, fore, s.elbows[side]);
            p[j.hand[side]] = hand;
        }
        p
    }
}

fn v(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(x, y, z)
}

/// Клип без ключей: на всё тело, руки держат оружие (остальное — `AnimClip { … }`).
fn clip(key: &str, about: &str, looped: bool, length: f32, speed: f32, stride: f32) -> AnimClip {
    AnimClip {
        key: key.into(),
        about: about.into(),
        source: OWN.into(),
        looped,
        length,
        speed,
        speed_spread: 0.0,
        stride,
        body: Body::Full,
        hands: Hands::Weapon,
        repeat: None,
        keys: Vec::new(),
    }
}

fn key(m: &CharModel, time: f32, pose: &Pose, events: Vec<Event>) -> Key {
    // Ничего не уходит под землю: у процедурной перезарядки лёжа кисть ныряла вниз.
    let mut pose = pose.clone();
    for p in &mut pose {
        p.y = p.y.max(0.5);
    }
    // Длины отрезков — как у модели: в файле ровно то, что сыграет игра (процедурные
    // действия чуть сдвигают голову и тянут руки).
    let sk = m.skeleton().expect("скелет");
    let points = m.points_of(&sk.pose(&sk.frame_of(&pose))).expect("скелет");
    Key { time, points: Points::from_array(points), weapon: None, events }
}

/// Оружие из матрицы (`Hold::frame`): рукоять, ствол, поворот.
fn weapon_key(frame: Mat4) -> WeaponKey {
    let z = frame.z_axis.truncate().normalize_or(Vec3::X);
    let y = frame.y_axis.truncate().normalize_or(Vec3::Y);
    let up0 = (Vec3::Y - z * z.dot(Vec3::Y)).normalize_or(Vec3::Y);
    let roll = z.dot(up0.cross(y)).atan2(up0.dot(y));
    let g = frame.w_axis.truncate() / VOXEL;
    let r = |x: f32| (x * 1000.0).round() / 1000.0;
    WeaponKey { grip: [r(g.x), r(g.y), r(g.z)], aim: [r(z.x), r(z.y), r(z.z)], roll: r(roll) }
}

impl CharModel {
    /// Стойка «стоит»: ноги чуть врозь, колени мягкие, вес чуть на заднюю.
    fn still_shape(&self, breath: f32) -> Shape {
        Shape {
            hips: v(-0.3, 27.6 - 0.25 * breath, -0.4),
            pitch: 0.04 + 0.012 * breath,
            yaw: 0.0,
            head_up: 0.03,
            feet: [v(-3.5, 0.5, 6.0), v(3.5, 0.5, -6.8)],
            knees: [v(1.0, 0.0, 0.1), v(1.0, 0.0, -0.1)],
            hands: [v(11.0, 40.0, 3.5), v(17.0, 41.0, -3.5)],
            elbows: [v(-0.2, -1.0, 0.7), v(0.0, -1.0, -0.6)],
        }
    }

    /// Присед на правое колено: левая стопа впереди, правое колено на земле, корпус
    /// наклонён к оружию.
    fn crouch_shape(&self, breath: f32) -> Shape {
        Shape {
            // Таз ~0,4 м (у RWR 0,37), корпус к оружию.
            hips: v(-3.0, 12.8 - 0.3 * breath, -0.3),
            pitch: 0.4 + 0.015 * breath,
            yaw: 0.0,
            head_up: 0.3,
            feet: [v(-17.0, 0.5, 5.5), v(8.0, 0.5, -7.0)],
            knees: [v(0.2, -1.0, 0.1), v(1.0, 0.7, -0.15)],
            hands: [v(9.0, 27.0, 3.5), v(15.0, 28.0, -3.5)],
            elbows: [v(-0.2, -1.0, 0.7), v(0.0, -1.0, -0.6)],
        }
    }

    /// Лёжа: корпус почти горизонтально от груди назад, голова приподнята, опора на
    /// локти; правая нога прямо, левая — чуть в сторону, «лягушкой».
    fn prone_shape(&self, breath: f32) -> Shape {
        Shape {
            hips: v(-15.0, 4.6 + 0.2 * breath, -0.3),
            pitch: 1.40,
            yaw: 0.0,
            head_up: 0.95,
            feet: [v(-41.0, 1.5, 7.0), v(-36.5, 1.8, -14.0)],
            knees: [v(0.0, -1.0, 0.3), v(0.0, -0.6, -1.0)],
            hands: [v(10.0, 6.0, 3.0), v(18.0, 5.5, -3.0)],
            elbows: [v(0.0, -1.0, 0.5), v(0.0, -1.0, -0.5)],
        }
    }

    /// Все клипы, которых ещё нет в `dir` (с `force` — все). Возвращает сохранённые.
    pub fn make_clips(&self, dir: &Path, force: &[String]) -> Result<Vec<String>, String> {
        let j = self.joints.ok_or("у модели нет скелета")?;
        let mut made = Vec::new();
        let (existing, _) = crate::anim::load_dir(dir);
        let mut save = |c: AnimClip| -> Result<(), String> {
            if existing.contains_key(&c.key) && !force.iter().any(|k| k == &c.key || k == "all") {
                return Ok(());
            }
            c.check()?;
            let path = c.save(dir)?;
            made.push(format!("{} ({} ключей, {:.2} с)", path.display(), c.keys.len(), c.seconds()));
            Ok(())
        };

        // --- Из записей движений (один раз, из старых файлов) ---
        // Шаг — свой, по устройству RWR (до 2026-11-07 — из записи CMU).
        save(self.walk_cycle(j))?;
        // Бег — свой, по устройству RWR (до 2026-11-07 — из записи CMU 35_17).
        save(self.run_cycle(j))?;
        // Бросок собирается процедурным действием поверх записи (если она загружена) —
        // как было в игре.
        let record = Clip::load(&dir.join("throw.ron")).ok();
        save(self.throw_clip(j, record.as_ref()))?;

        // --- Стоит ---
        let mut c = clip("still", "стоит с оружием, дышит", true, 3.0, 1.0, 0.0);
        for (t, b) in [(0.0, 0.0), (1.4, 1.0), (2.2, 0.6)] {
            c.keys.push(key(self, t, &self.shaped(j, &self.still_shape(b)), vec![]));
        }
        save(c)?;

        // --- Присед ---
        let mut c = clip("crouch still", "присел на правое колено, оружие у плеча", true, 2.4, 1.0, 0.0);
        for (t, b) in [(0.0, 0.0), (1.1, 1.0), (1.8, 0.5)] {
            c.keys.push(key(self, t, &self.shaped(j, &self.crouch_shape(b)), vec![]));
        }
        save(c)?;
        save(self.crouch_walk(j))?;

        // --- Лёжа, ползком, лечь и встать ---
        let mut c = clip("prone", "лежит, опора на локти, оружие вперёд", true, 2.6, 1.0, 0.0);
        for (t, b) in [(0.0, 0.0), (1.3, 1.0), (2.0, 0.4)] {
            c.keys.push(key(self, t, &self.shaped(j, &self.prone_shape(b)), vec![]));
        }
        save(c)?;
        save(self.crawl(j))?;
        let down = self.going_prone(j);
        let mut up = down.clone();
        up.key = "leaving prone".into();
        up.about = "встаёт из положения лёжа в присед (тот же путь, что «лечь», обратно)".into();
        let len = up.length;
        up.keys.reverse();
        for k in &mut up.keys {
            k.time = ((len - k.time) * 1000.0).round() / 1000.0;
        }
        save(down)?;
        save(up)?;

        // --- Перезарядки: стоя (по видам) и лёжа ---
        let stand = self.pose(Motion::default());
        for (kind, k, about) in [
            (ReloadKind::Magazine, "reloading", "перезарядка магазином: отстегнуть, подсумок, вставить, затвор"),
            (ReloadKind::Cylinder, "reloading, cylinder", "барабан: открыть, по выстрелу в гнездо, закрыть"),
            (ReloadKind::Pump, "reloading, pump", "трубчатый магазин: по выстрелу снизу, цевьё"),
            (ReloadKind::Break, "reloading, break", "переломить, вложить выстрел, закрыть"),
        ] {
            save(self.reload_clip(j, &stand, kind, k, about))?;
        }
        let prone = self.prone_with_weapon(j);
        save(self.reload_clip(j, &prone, ReloadKind::Magazine, "reloading, prone", "перезарядка лёжа: оружие к себе, магазин, затвор"))?;
        Ok(made)
    }

    /// Клип стандарта из записи движений (`wind_lab --import-mocap`): плотные кадры записи
    /// → редкие ключи, метки шагов — где стопа встала.
    pub fn clip_from_record(&self, rec: &Clip, key_name: &str, about: &str) -> Result<AnimClip, String> {
        let j = self.joints.ok_or("у модели нет скелета")?;
        let mut c = self.record_clip(j, rec, key_name, about)?;
        // У действия (бросок) — метка выпуска на пике скорости кисти, руки — из клипа.
        if !rec.cycle {
            c.hands = Hands::Clip;
            let at = rec.peak * c.length;
            if let Some(k) = c.keys.iter_mut().min_by(|a, b| (a.time - at).abs().total_cmp(&(b.time - at).abs())) {
                k.events.push(Event::Release);
            }
        }
        Ok(c)
    }

    /// Круг из записи движений: плотные кадры → ключи, метки шагов — где стопа встала.
    fn record_clip(&self, _j: Joints, rec: &Clip, key_name: &str, about: &str) -> Result<AnimClip, String> {
        let sk = self.skeleton().ok_or("у модели нет скелета")?;
        let rec = rec.uniform();
        let n = rec.frames.len();
        let mut c = clip(key_name, about, rec.cycle, rec.duration, 1.0, rec.stride);
        c.source = format!("{}; ключи — из записи (wind_lab --make-clips)", rec.source);
        let poses: Vec<Pose> = rec.frames.iter().map(|f| sk.pose(f)).collect();
        let j = self.joints.ok_or("нет скелета")?;
        for (i, p) in poses.iter().enumerate() {
            let mut events = Vec::new();
            // Стопа встала: была выше земли, а в этом кадре — ниже порога.
            for (side, s) in [(0, Side::Right), (1, Side::Left)] {
                let low = poses.iter().map(|q| q[j.foot[side]].y).fold(f32::MAX, f32::min);
                let prev = &poses[(i + n - 1) % n];
                if p[j.foot[side]].y < low + 1.2 && prev[j.foot[side]].y >= low + 1.2 {
                    events.push(Event::Step(s));
                }
            }
            c.keys.push(key(self, rec.duration * i as f32 / n as f32, p, events));
        }
        c.simplify(0.6);
        Ok(c)
    }

    /// Приседом вперёд: круг из двух шагов, корпус наклонён, таз пружинит.
    fn crouch_walk(&self, j: Joints) -> AnimClip {
        // Круг 0,67 с (у RWR так же): при скорости приседом — путь за круг.
        let seconds = 0.8 / 1.2;
        let leg = self.skeleton().map_or(28.0, |s| s.leg);
        let stride_vox = CROUCH_SPEED * seconds / VOXEL;
        let mut c = clip("crouching forwards", "идёт приседом, корпус к оружию", true, 0.8, 1.2, stride_vox / leg);
        let reach = stride_vox * 0.25;
        for k in 0..8 {
            let ph = TAU * k as f32 / 8.0;
            let foot = |ph: f32, z: f32| v(reach * ph.sin(), 0.5 + 4.5 * ph.cos().max(0.0), z);
            let s = Shape {
                hips: v(0.5, 21.5 + 0.7 * (2.0 * ph).cos(), -0.3),
                // Корпус сильно вперёд, голова на ~1,25 м (у RWR — 1,2 м): бегом пригнувшись.
                pitch: 0.88,
                yaw: 0.06 * ph.sin(),
                head_up: 0.55,
                feet: [foot(ph, 5.5), foot(ph + PI, -6.0)],
                knees: [v(1.0, 0.4, 0.1), v(1.0, 0.4, -0.1)],
                hands: [v(12.0, 33.0, 3.5), v(18.0, 34.0, -3.5)],
                elbows: [v(-0.2, -1.0, 0.7), v(0.0, -1.0, -0.6)],
            };
            // Стопа встаёт, когда вынос вперёд закончился (sin = 1).
            let events = match k {
                2 => vec![Event::Step(Side::Right)],
                6 => vec![Event::Step(Side::Left)],
                _ => vec![],
            };
            c.keys.push(key(self, 0.1 * k as f32, &self.shaped(j, &s), events));
        }
        c
    }

    /// Ползком, как у RWR по устройству: сторона за стороной — правая рука тянется далеко
    /// вперёд вместе с подтянутым вбок правым коленом («лягушкой»), потом то же слева; два
    /// толчка за круг. Размахи — по разбору RWR (колено почти к тазу и на ~0,55 м вбок,
    /// кисть на ~1 м вперёд от таза, локти широко), кривые — свои.
    fn crawl(&self, j: Joints) -> AnimClip {
        let seconds = 0.4 / 0.45;
        let leg = self.skeleton().map_or(28.0, |s| s.leg);
        let stride_vox = PRONE_SPEED * seconds / VOXEL;
        let mut c = AnimClip {
            body: Body::Full,
            hands: Hands::Clip,
            ..clip("prone, moving forwards", "ползёт: рука и колено одной стороны вперёд, потом другой; два толчка за круг", true, 0.4, 0.45, stride_vox / leg)
        };
        let base = self.prone_shape(0.0);
        let n = 12;
        for k in 0..n {
            let u = k as f32 / n as f32;
            let ph = TAU * u;
            // Доля «правая сторона вперёд»: 1 — правая рука вытянута и правое колено
            // подтянуто (u = 0,25), 0 — то же слева (u = 0,75).
            let right = 0.5 + 0.5 * ph.sin();
            let mut sh = base;
            let hip = base.hips;
            sh.hips = v(hip.x + 0.8 * (2.0 * ph).sin(), hip.y + 0.35 * (1.0 - (2.0 * ph).cos()), hip.z);
            // Плечо тянущейся руки — вперёд (плюс — левое плечо вперёд).
            sh.yaw = -0.09 * ph.sin();
            let h = sh.hips;
            for (side, sign, w) in [(0, 1.0, right), (1, -1.0, 1.0 - right)] {
                // Рука, которая идёт вперёд, приподнята над землёй.
                let forward = (sign * ph.cos()).max(0.0);
                let near = v(h.x + 20.0, 0.8, h.z + 8.5 * sign);
                let far = v(h.x + 31.0, 1.5, h.z + 4.0 * sign);
                sh.hands[side] = near.lerp(far, w) + v(0.0, 2.2 * forward, 0.0);
                sh.elbows[side] = v(0.0, -0.6, sign);
                // Нога: вытянута назад — или подтянута коленом вбок, стопа под коленом.
                let stretched = v(h.x - 28.0, 2.0, h.z + 6.0 * sign);
                let drawn = v(h.x - 17.0, 1.5, h.z + 9.5 * sign);
                sh.feet[side] = stretched.lerp(drawn, w);
                sh.knees[side] = v(0.0, -0.5, 0.3 * sign).lerp(v(0.15, -0.15, sign), w);
            }
            let events = if k == 3 || k == 9 { vec![Event::Push] } else { vec![] };
            c.keys.push(key(self, (0.4 * u * 1000.0).round() / 1000.0, &self.shaped(j, &sh), events));
        }
        c
    }

    /// Шаг с оружием у плеча, по устройству как у RWR — «тактический»: таз почти не
    /// пружинит, корпус прямой, оружие не качается (руки ставит прицел, `Hands::Weapon`);
    /// круг 0,8 с, опора — больше половины круга, стопа поднимается невысоко (~0,25 м).
    /// Путь за круг — под шаг 2,2 м/с: стопы не скользят. Кривые — свои.
    fn walk_cycle(&self, j: Joints) -> AnimClip {
        let length = 0.8;
        let leg = self.skeleton().map_or(28.0, |s| s.leg);
        let stride_vox = 2.2 * length / VOXEL;
        let mut c = clip("walking", "шаг с оружием у плеча: круг — два шага, корпус ровно", true, length, 1.0, stride_vox / leg);
        let ground = 0.58;
        let land = stride_vox * ground * 0.45;
        let off = land - stride_vox * ground;
        // Полёт стопы: (доля круга, вперёд, высота) — отрыв носка, колено вперёд, вынос, постановка.
        let swing = [(ground, off, 0.5), (0.68, off + 6.0, 7.5), (0.8, -1.0, 9.5), (0.92, land - 2.0, 4.0), (1.0, land, 0.5)];
        let foot_at = |p: f32| -> (f32, f32) {
            let p = p.rem_euclid(1.0);
            if p < ground {
                return (land + (off - land) * p / ground, 0.5);
            }
            let i = swing.windows(2).position(|w| p <= w[1].0).unwrap_or(swing.len() - 2);
            let at = |k: isize| swing[k.clamp(0, swing.len() as isize - 1) as usize];
            let (p0, p1, p2, p3) = (at(i as isize - 1), at(i as isize), at(i as isize + 1), at(i as isize + 2));
            let t = (p - p1.0) / (p2.0 - p1.0).max(1e-4);
            let cr = |a: f32, b: f32, c: f32, d: f32| 0.5 * (2.0 * b + (c - a) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t * t + (3.0 * b - a - 3.0 * c + d) * t * t * t);
            (cr(p0.1, p1.1, p2.1, p3.1), cr(p0.2, p1.2, p2.2, p3.2).max(0.5))
        };
        let n = 12;
        for k in 0..n {
            let u = k as f32 / n as f32;
            let ph = TAU * u;
            let (rx, ry) = foot_at(u);
            let (lx, ly) = foot_at(u + 0.5);
            let s = Shape {
                // Таз почти ровно: чуть выше над опорной ногой посередине опоры.
                hips: v(0.0, 28.0 - 0.35 * (2.0 * ph).cos(), -0.4 + 0.6 * (ph - 0.6).cos()),
                pitch: 0.07,
                yaw: 0.04 * ph.cos(),
                head_up: 0.04,
                feet: [v(rx, ry, 4.5), v(lx, ly, -5.0)],
                knees: [v(1.0, 0.1, 0.05), v(1.0, 0.1, -0.05)],
                hands: [v(9.0, 43.0, 3.0), v(16.0, 43.5, -1.0)],
                elbows: [v(-0.2, -1.0, 0.7), v(0.0, -1.0, -0.6)],
            };
            let events = match k {
                0 => vec![Event::Step(Side::Right)],
                6 => vec![Event::Step(Side::Left)],
                _ => vec![],
            };
            c.keys.push(key(self, (length * u * 1000.0).round() / 1000.0, &self.shaped(j, &s), events));
        }
        c
    }

    /// Бег с оружием, по устройству как у RWR (разбор — «Как устроены анимации RWR»):
    /// - круг 0,56 с (0,9 при скорости 1,6), опора — треть круга; после толчка пятка
    ///   взлетает высоко позади, стопа идёт вперёд высоко, колено выносится до уровня ~0,65 м;
    /// - корпус наклонён сильно (шея на ~0,3 м впереди таза), таз пружинит на 0,1 м и уходит
    ///   к опорной ноге, плечи опускаются вместе с тазом;
    /// - **руки**: оружие низко, у пояса, наискось поперёк корпуса (правая кисть ближе, левая
    ///   дальше и левее) и в такт шагу уходит вбок: правая нога впереди — влево, левая — вправо;
    ///   локти широко. Руки — из клипа (`Hands::Clip`), пока солдат не целится.
    ///
    /// Кривые и ключи — свои, числа (длительности, высоты, размахи) — по разбору.
    fn run_cycle(&self, j: Joints) -> AnimClip {
        let (length, speed) = (0.9, 1.6);
        let seconds = length / speed;
        let leg = self.skeleton().map_or(28.0, |s| s.leg);
        let stride_vox = 6.0 * seconds / VOXEL;
        let mut c = AnimClip { hands: Hands::Clip, ..clip("running", "бег с оружием у пояса: круг — два шага, высокое колено, оружие качается в такт", true, length, speed, stride_vox / leg) };
        // Опора — треть круга, как у RWR: стопа проходит чуть меньше, чем тело (у RWR
        // бег медленнее), — на 0,1 м проскальзывает, сверху не видно.
        let ground = 0.32;
        let (land, off) = (11.0, -16.0);
        // Полёт стопы: (доля круга, вперёд, высота) — толчок; стопа ещё уходит назад и вверх
        // и выше всего позади, когда встаёт другая нога (0,5); потом высоко под тазом,
        // колено вперёд, постановка.
        let swing = [(ground, off, 0.5), (0.5, -24.0, 20.0), (0.67, -5.0, 17.0), (0.83, 3.0, 16.5), (0.93, 10.0, 5.0), (1.0, land, 0.5)];
        let foot_at = |p: f32| -> (f32, f32) {
            let p = p.rem_euclid(1.0);
            if p < ground {
                return (land + (off - land) * p / ground, 0.5);
            }
            // Катмулл — Ром по точкам полёта: плавная дуга без углов.
            let i = swing.windows(2).position(|w| p <= w[1].0).unwrap_or(swing.len() - 2);
            let at = |k: isize| swing[k.clamp(0, swing.len() as isize - 1) as usize];
            let (p0, p1, p2, p3) = (at(i as isize - 1), at(i as isize), at(i as isize + 1), at(i as isize + 2));
            let t = (p - p1.0) / (p2.0 - p1.0).max(1e-4);
            let cr = |a: f32, b: f32, c: f32, d: f32| 0.5 * (2.0 * b + (c - a) * t + (2.0 * a - 5.0 * b + 4.0 * c - d) * t * t + (3.0 * b - a - 3.0 * c + d) * t * t * t);
            (cr(p0.1, p1.1, p2.1, p3.1), cr(p0.2, p1.2, p2.2, p3.2).max(0.5))
        };
        let n = 12;
        for k in 0..n {
            let u = k as f32 / n as f32;
            let ph = TAU * u;
            let (rx, ry) = foot_at(u);
            let (lx, ly) = foot_at(u + 0.5);
            // Таз: выше всего — когда стопа встаёт, ниже всего — посередине опоры; назад
            // посередине опоры; вбок — к опорной ноге (правая — первая половина круга).
            let hip_y = 27.6 + 1.8 * (2.0 * ph).cos();
            let hips = v(-1.4 * (1.0 - (2.0 * ph).cos()), hip_y, -0.3 - 1.7 + 1.7 * (ph - 0.5).cos());
            // Оружие у пояса, в такт шагу вбок: правая нога впереди (u = 0) — влево.
            let swing_z = -(ph).cos();
            let hand_y = 35.0 - 0.4 * (hip_y - 27.6);
            let s = Shape {
                hips,
                pitch: 0.5 + 0.05 * (1.0 - (2.0 * ph).cos()),
                yaw: 0.12 * ph.cos(),
                head_up: 0.45,
                feet: [v(rx, ry, 4.0), v(lx, ly, -4.8)],
                knees: [v(1.0, 0.2, 0.05), v(1.0, 0.2, -0.05)],
                hands: [v(hips.x + 14.0, hand_y, hips.z + 3.0 + 4.0 * swing_z), v(hips.x + 18.5, hand_y - 1.0, hips.z - 4.5 + 4.5 * swing_z)],
                elbows: [v(-0.3, -1.0, 0.9), v(0.1, -1.0, -0.8)],
            };
            let events = match k {
                0 => vec![Event::Step(Side::Right)],
                6 => vec![Event::Step(Side::Left)],
                _ => vec![],
            };
            c.keys.push(key(self, (length * u * 1000.0).round() / 1000.0, &self.shaped(j, &s), events));
        }
        c
    }

    /// Лечь из приседа: руки к земле, колени вниз, тело вытягивается назад от груди.
    fn going_prone(&self, j: Joints) -> AnimClip {
        let mut c = AnimClip { body: Body::Full, hands: Hands::Clip, ..clip("going prone", "ложится из приседа: руки к земле, тело назад", false, 0.55, 1.0, 0.0) };
        let start = self.crouch_shape(0.0);
        let mut hands_down = start;
        hands_down.hips = v(-4.0, 12.5, -0.3);
        hands_down.pitch = 0.95;
        hands_down.head_up = 0.5;
        hands_down.feet = [v(-17.0, 1.0, 5.5), v(-12.0, 1.0, -6.5)];
        hands_down.knees = [v(0.3, -1.0, 0.1), v(0.3, -1.0, -0.1)];
        hands_down.hands = [v(13.0, 2.0, 6.0), v(14.0, 2.0, -6.0)];
        let mut stretch = self.prone_shape(0.0);
        stretch.hips = v(-11.0, 8.0, -0.3);
        stretch.pitch = 1.2;
        stretch.feet = [v(-33.0, 1.5, 6.5), v(-31.0, 2.0, -9.0)];
        stretch.hands = [v(13.0, 2.0, 5.0), v(15.0, 2.0, -5.0)];
        let end = self.prone_with_weapon(j);
        c.keys.push(key(self, 0.0, &self.with_weapon(self.shaped(j, &start)), vec![Event::WeakHand(false)]));
        c.keys.push(key(self, 0.2, &self.shaped(j, &hands_down), vec![]));
        c.keys.push(key(self, 0.38, &self.shaped(j, &stretch), vec![]));
        c.keys.push(key(self, 0.55, &end, vec![Event::WeakHand(true)]));
        c
    }

    /// Лёжа с оружием в руках — как в игре (руки ставит `pose`).
    fn prone_with_weapon(&self, j: Joints) -> Pose {
        let mut p = self.shaped(j, &self.prone_shape(0.0));
        self.hold_weapon(&mut p, j, Motion { prone: 1.0, ..Default::default() }, 0.0, true);
        p
    }

    /// Перезарядка: процедурное действие (`actions.rs`) поверх позы `base` — в ключи.
    fn reload_clip(&self, _j: Joints, base: &Pose, kind: ReloadKind, key_name: &str, about: &str) -> AnimClip {
        let mut c = AnimClip { body: Body::Upper, hands: Hands::Clip, ..clip(key_name, about, false, 1.0, 1.0, 0.0) };
        c.source = "свой: процедурная перезарядка (character/actions.rs), переведена в ключи".into();
        // По выстрелу — отрезок повторяется (рука к подсумку и обратно).
        c.repeat = match kind {
            ReloadKind::Cylinder => Some((0.15, 0.8)),
            ReloadKind::Pump => Some((0.12, 0.76)),
            _ => None,
        };
        let events = |t: f32| -> Vec<Event> {
            let at = |x: f32| (t - x).abs() < 1e-4;
            let mut e = Vec::new();
            match kind {
                ReloadKind::Magazine => {
                    if at(0.14) {
                        e.push(Event::WeakHand(false));
                    }
                    if at(0.34) {
                        e.push(Event::Magazine(false));
                    }
                    if at(0.78) {
                        e.push(Event::Magazine(true));
                    }
                    if at(0.9) {
                        e.push(Event::Cycle);
                    }
                }
                ReloadKind::Pump => {
                    if at(0.0) {
                        e.push(Event::WeakHand(false));
                    }
                    if at(0.9) {
                        e.push(Event::Cycle);
                    }
                }
                _ => {
                    if at(0.0) {
                        e.push(Event::WeakHand(false));
                    }
                }
            }
            if at(1.0) {
                e.push(Event::WeakHand(true));
            }
            e
        };
        let n = 60;
        let mut times: Vec<f32> = (0..=n).map(|i| i as f32 / n as f32).collect();
        times.extend([0.14, 0.34, 0.78, 0.9]);
        times.sort_by(f32::total_cmp);
        times.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
        for t in times {
            let a = self.act_procedural(base, Action::Reload { kind, t, rounds: 1 });
            let mut k = key(self, (t * 1000.0).round() / 1000.0, &a.pose, events(t));
            k.weapon = a.weapon.map(weapon_key);
            c.keys.push(k);
        }
        c.simplify(0.5);
        c
    }

    /// Бросок гранаты: процедурное действие поверх «стоит» — в ключи.
    fn throw_clip(&self, _j: Joints, record: Option<&Clip>) -> AnimClip {
        let base = self.pose(Motion::default());
        let mut c = AnimClip { body: Body::Upper, hands: Hands::Clip, ..clip("throwing", "бросок гранаты: винтовка в левую руку, замах из-за плеча, бросок через верх", false, THROW_TIME, 1.0, 0.0) };
        c.source = match record {
            Some(r) => format!("{}; корпус и правая рука — из записи, винтовка — процедурно; переведено в ключи", r.source),
            None => "свой: процедурный бросок (character/actions.rs), переведён в ключи".into(),
        };
        // Кадры записи — на этой модели.
        let rec: Option<(Vec<Pose>, f32)> = record.and_then(|r| {
            let sk = self.skeleton()?;
            let r = r.uniform();
            Some((r.frames.iter().map(|f| sk.pose(f)).collect(), r.peak))
        });
        let n = 45;
        let mut times: Vec<f32> = (0..=n).map(|i| THROW_TIME * i as f32 / n as f32).collect();
        times.extend([0.04, THROW_RELEASE]);
        times.sort_by(f32::total_cmp);
        times.dedup_by(|a, b| (*a - *b).abs() < 1e-4);
        for t in times {
            let a = match &rec {
                Some((frames, peak)) => self.throw_over_record(&base, frames, *peak, t),
                None => self.act_procedural(&base, Action::Throw { t }),
            };
            let mut events = Vec::new();
            if (t - 0.04).abs() < 1e-4 {
                events.push(Event::StrongHand(false));
            }
            if (t - THROW_RELEASE).abs() < 1e-4 {
                events.push(Event::Release);
            }
            if (t - THROW_TIME).abs() < 1e-4 {
                events.push(Event::StrongHand(true));
            }
            let mut k = key(self, (t * 1000.0).round() / 1000.0, &a.pose, events);
            k.weapon = a.weapon.map(weapon_key);
            c.keys.push(k);
        }
        c.simplify(0.5);
        c
    }
}
