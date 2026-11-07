//! Свои солдаты: модели RWR со скелетом, процедурная поза, оружие. Тела убитых —
//! в `ragdoll`.
//!
//! У каждой палки скелета свои воксели и своя матрица; матрицы всех палок солдата
//! уходят в видеокарту, и модель рисуется одним вызовом на всех солдат (как у RWR).
//! Воксель у сустава смешивает положение между своей палкой и соседней — сгиб плавный.
//! Как у RWR, солдат рисуется облаком точек: каждый воксель — квадратик постоянного
//! размера на экране (см. заметку «Как устроена картинка RWR»). Грани, склеенные
//! по форме, остались только для теней.

mod actions;
mod clips;
mod gait;
mod make;

pub use actions::{Acted, Action, ReloadKind, THROW_RELEASE, THROW_TIME};
pub use clips::{Built, WeaponAt};
pub use gait::Skeleton;
pub use make::{CROUCH_SPEED, PRONE_SPEED};
use crate::ragdoll::Rig;
use crate::rwr::{self, RwrModel, VOXEL};
use wind_sim::arsenal::Arsenal;
use bytemuck::{Pod, Zeroable};
use glam::{IVec3, Mat3, Mat4, Quat, Vec2, Vec3};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Вершина меша для теней: позиция в позе покоя (м) и номер палки.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct CharVertex {
    pub pos: [f32; 3],
    pub bone: u32,
}

/// Угол точки-вокселя: центр вокселя в позе покоя (м) и цвет sRGB; в `color[3]` —
/// номер угла квадратика 0..3 и уровень детализации (биты 2..3). В `bones` — палка
/// вокселя, соседняя палка у ближайшего сустава и расстояние до этого сустава
/// в четвертях вокселя (255 — далеко). На каждый воксель — 4 вершины.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct PointVertex {
    pub pos: [f32; 3],
    pub color: [u8; 4],
    pub bones: [u8; 4],
}

/// Меш модели целиком (солдата или оружия).
#[derive(Clone, Debug, Default)]
pub struct CharMesh {
    /// Грани для теней.
    pub vertices: Vec<CharVertex>,
    pub indices: Vec<u32>,
    /// Точки для картинки, по уровням детализации: на уровне n воксели объединены
    /// кубиками 2ⁿ×2ⁿ×2ⁿ. По 4 вершины и 6 индексов на точку.
    pub lods: [PointMesh; LODS],
}

/// Сколько уровней детализации у точек солдата.
pub const LODS: usize = 3;

#[derive(Clone, Debug, Default)]
pub struct PointMesh {
    pub points: Vec<PointVertex>,
    pub indices: Vec<u32>,
}

/// Номера нужных точек скелета. Без них модель не анимируется — стоит как есть.
#[derive(Clone, Copy, Debug)]
struct Joints {
    head: usize,
    neck: usize,
    mid: usize,
    shoulder: [usize; 2],
    elbow: [usize; 2],
    hand: [usize; 2],
    hip: [usize; 2],
    knee: [usize; 2],
    foot: [usize; 2],
}

pub struct CharModel {
    pub name: String,
    pub rest: Vec<Vec3>,
    /// Палки скелета: (начало, конец). На каждую — своя матрица.
    pub sticks: Vec<(usize, usize)>,
    /// Меш всей модели в `CharacterSet::meshes`.
    pub mesh: usize,
    joints: Option<Joints>,
    /// Поворот корпуса в позе покоя (см. `body_frame`): по бёдрам и по плечам.
    rest_body: Mat3,
    rest_upper: Mat3,
    /// Тело для физики — когда солдата убьют.
    pub(crate) rig: Rig,
    /// Клипы стандарта анимаций, собранные на этой модели: ключ → клип
    /// (`data/animations`, см. `crate::anim`). Нет клипа — движение процедурное.
    pub clips: HashMap<String, Built>,
}

pub struct WeaponModel {
    pub name: String,
    pub mesh: usize,
    /// Как перезаряжается, сколько секунд и сколько выстрелов в магазине — для анимации.
    pub reload: ReloadKind,
    pub reload_time: f32,
    pub rounds: u32,
}

/// Всё про своих солдат: меши, модели, кто в какой команде.
pub struct CharacterSet {
    pub meshes: Vec<CharMesh>,
    pub models: Vec<CharModel>,
    pub weapons: Vec<WeaponModel>,
    /// Для каждой команды по наборам из `arsenal`: (модель, оружие).
    pub teams: [Vec<(usize, usize)>; 2],
    pub team_names: [String; 2],
    /// Оружие и классы — те же, что у симуляции: набор солдата выбирается одинаково.
    pub arsenal: Arsenal,
}

/// Поза — положения 15 точек скелета в пространстве модели (воксели, +X вперёд).
pub type Pose = Vec<Vec3>;

impl CharacterSet {
    /// Читает `data/weapons.ron`, `data/factions.ron` и все упомянутые там модели.
    pub fn load(data_dir: &Path) -> Result<Self, String> {
        let arsenal = Arsenal::load(data_dir)?;
        let mut builder = Builder::default();
        let mut set_models: Vec<(String, usize)> = Vec::new();
        let mut set_weapons: Vec<(String, usize)> = Vec::new();
        let mut models = Vec::new();
        let mut weapons = Vec::new();
        let mut teams: [Vec<(usize, usize)>; 2] = Default::default();
        let read = |name: &str| -> Result<RwrModel, String> {
            let p = data_dir.join("models").join(format!("{name}.xml"));
            let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            rwr::parse(&text).map_err(|e| format!("{}: {e}", p.display()))
        };
        for (team, kits) in arsenal.teams.iter().enumerate() {
            for kit in kits {
                let model = match set_models.iter().find(|(n, _)| *n == kit.model) {
                    Some(&(_, i)) => i,
                    None => {
                        models.push(builder.character(&kit.model, &read(&kit.model)?));
                        set_models.push((kit.model.clone(), models.len() - 1));
                        models.len() - 1
                    }
                };
                let path = &arsenal.weapons[kit.weapon].model;
                let weapon = match set_weapons.iter().find(|(n, _)| n == path) {
                    Some(&(_, i)) => i,
                    None => {
                        let spec = &arsenal.weapons[kit.weapon];
                        let mesh = builder.prop(&read(path)?, spec.grip);
                        weapons.push(WeaponModel {
                            name: path.clone(),
                            mesh,
                            reload: ReloadKind::from_name(&spec.reload),
                            reload_time: spec.reload_time,
                            rounds: spec.magazine_size,
                        });
                        set_weapons.push((path.clone(), weapons.len() - 1));
                        weapons.len() - 1
                    }
                };
                teams[team].push((model, weapon));
            }
        }
        // Клипы стандарта анимаций — на каждой модели с её длинами.
        let (clips, _old) = crate::anim::load_dir(&data_dir.join("animations"));
        for m in &mut models {
            m.clips = clips.values().filter_map(|c| Some((c.key.clone(), m.build_clip(c)?))).collect();
        }
        Ok(CharacterSet { meshes: builder.meshes, models, weapons, teams, team_names: arsenal.team_names.clone(), arsenal })
    }

    /// Радиан фазы шага на метр пути при скорости `speed`: по длине круга шага и бега
    /// (у всех моделей скелет один), а без клипов — `stride_rate`.
    pub fn stride_rate(&self, speed: f32) -> f32 {
        self.stride_rate_in(speed, 0.0, false)
    }

    /// То же для стойки: присед (`crouch` 0..1) и лёжа — по длине своих кругов.
    pub fn stride_rate_in(&self, speed: f32, crouch: f32, prone: bool) -> f32 {
        let Some(m) = self.models.first() else { return stride_rate(speed) };
        let stride = |k: &str| m.clips.get(k).map(|c| c.stride).filter(|&s| s > 0.01);
        let standing = match (stride("walking"), stride("running")) {
            (Some(w), Some(r)) => w + (r * gait::RUN_STRIDE - w) * run_share(speed),
            _ => return stride_rate(speed),
        };
        let stride = if prone {
            stride("prone, moving forwards").unwrap_or(standing)
        } else {
            let low = stride("crouching forwards").unwrap_or(standing);
            standing + (low - standing) * crouch.clamp(0.0, 1.0)
        };
        std::f32::consts::TAU / stride.max(0.1)
    }

    /// Какая модель и оружие у солдата — тот же набор, что выбрала симуляция.
    pub fn pick(&self, team: u8, id: u32) -> Option<(usize, usize)> {
        self.teams[team as usize & 1].get(self.arsenal.pick(team, id)).copied()
    }
}

// --- Поза --------------------------------------------------------------------

/// Состояние для анимации.
#[derive(Clone, Copy, Debug, Default)]
pub struct Motion {
    /// Скорость, м/с.
    pub speed: f32,
    /// Фаза шага, рад: полный круг — два шага. Растёт на `stride_rate` за метр пути.
    pub phase: f32,
    /// Куда идёт относительно взгляда: x — вперёд, y — вправо (ось z модели).
    /// Ноль — вперёд.
    pub dir: Vec2,
    /// Как держит оружие: 0 — целится (ствол по взгляду), 1 — несёт на бегу поперёк
    /// груди, как в RWR.
    pub carry: f32,
    /// Насколько присел: 0 — стоит, 1 — присел (плавно, пока садится и встаёт).
    pub crouch: f32,
    /// Насколько лёг: 0 — не лежит, 1 — лежит; между ними — ложится или встаёт
    /// (клипы «going prone» / «leaving prone» на этой доле).
    pub prone: f32,
    /// Встаёт из положения лёжа (а не ложится).
    pub rising: bool,
    /// Часы, с — для клипов на месте (дыхание): у каждого солдата свой сдвиг.
    pub clock: f32,
}

/// На сколько корпус развёрнут левым плечом вперёд, когда солдат целится, рад.
const BLADE: f32 = 0.6;
/// Лёжа рукоять — перед правым плечом, чуть ниже: оружие вдоль земли, на уровне головы.
const PRONE_GRIP: Vec3 = Vec3::new(7.5, -1.5, -3.5);

/// Насколько солдат бежит, а не идёт: 0 — шагом (до 2,6 м/с), 1 — бегом (от 4,5 м/с).
pub fn run_share(speed: f32) -> f32 {
    let u = ((speed - 2.6) / 1.9).clamp(0.0, 1.0);
    u * u * (3.0 - 2.0 * u)
}

/// Сколько радиан фазы шага на метр пути. Круг (два шага) шагом — 1,5 м, бегом — 2,5 м:
/// у RWR круг бега ≈ 0,56 с, при 4,5 м/с это около 2,5 м.
pub fn stride_rate(speed: f32) -> f32 {
    std::f32::consts::TAU / (1.5 + 1.0 * run_share(speed))
}

impl CharModel {
    /// Живой солдат: стоит, идёт или бежит, присел или лежит (клипы стандарта анимаций),
    /// руки держат оружие стволом вперёд. Ложится и встаёт — клипы «going prone» и
    /// «leaving prone»: они начинаются и кончаются приседом, из стойки в них — плавно.
    pub fn pose(&self, m: Motion) -> Pose {
        let Some(j) = self.joints else { return self.rest.clone() };
        let dir = m.dir.normalize_or(Vec2::X);
        if m.prone >= 1.0 {
            return self.prone_pose(m, j, dir);
        }
        let (mut p, twist) = self.upright(m, j, dir);
        self.hold_weapon(&mut p, j, Motion { prone: 0.0, ..m }, twist);
        if m.prone > 0.0 {
            let (key, u) = if m.rising { ("leaving prone", 1.0 - m.prone) } else { ("going prone", m.prone) };
            if let Some(clip) = self.clips.get(key) {
                // Ложась — из стойки в клип за первую четверть; вставая — в стойку за последнюю.
                let edge = |x: f32| {
                    let x = (x / 0.25).clamp(0.0, 1.0);
                    x * x * (3.0 - 2.0 * x)
                };
                let w = if m.rising { edge(1.0 - u) } else { edge(u) };
                let c = clip.at(u);
                for (v, c) in p.iter_mut().zip(c) {
                    *v = v.lerp(c, w);
                }
            }
        }
        p
    }

    /// Стоя или присев (по `crouch`): ноги и корпус из клипов, без них — процедурно.
    fn upright(&self, m: Motion, j: Joints, dir: Vec2) -> (Pose, f32) {
        let (moving, run) = ((m.speed / 1.2).clamp(0.0, 1.0), run_share(m.speed));
        let (stand, twist) = if m.crouch < 1.0 {
            match self.gait_pose(m.speed, m.phase, dir, m.clock) {
                Some(p) => (p, 0.0),
                None => (self.legs_procedural(m, j), 0.12 * run * moving * m.phase.sin()),
            }
        } else {
            (Vec::new(), 0.0)
        };
        if m.crouch <= 0.0 {
            return (stand, twist);
        }
        let Some(low) = self.cycle_pose(("crouch still", "crouching forwards"), CROUCH_SPEED * 0.6, &m, dir) else {
            return if stand.is_empty() { (self.legs_procedural(m, j), twist) } else { (stand, twist) };
        };
        if m.crouch >= 1.0 || stand.is_empty() {
            return (low, 0.0);
        }
        let k = m.crouch * m.crouch * (3.0 - 2.0 * m.crouch);
        (stand.iter().zip(&low).map(|(a, b)| a.lerp(*b, k)).collect(), twist * (1.0 - k))
    }

    /// Лёжа: на месте — оружие в руках, ползком — руки из клипа (на локтях).
    fn prone_pose(&self, m: Motion, j: Joints, dir: Vec2) -> Pose {
        let full = PRONE_SPEED * 0.6;
        let Some(crawl) = self.cycle_pose(("prone", "prone, moving forwards"), full, &m, dir) else {
            return self.rest.clone();
        };
        let mut held = crawl.clone();
        self.hold_weapon(&mut held, j, m, 0.0);
        let k = (m.speed / full).clamp(0.0, 1.0);
        held.iter().zip(&crawl).map(|(a, b)| a.lerp(*b, k)).collect()
    }

    /// Руки держат оружие поверх позы `p` (стоит, как в игре: стволом по взгляду) —
    /// для редактора анимаций: показать клип так, как он будет в игре.
    pub fn with_weapon(&self, mut p: Pose) -> Pose {
        if let Some(j) = self.joints {
            self.hold_weapon(&mut p, j, Motion::default(), 0.0);
        }
        p
    }

    /// Руки: правая у приклада, левая на цевье, ствол вперёд — туда, куда смотрит
    /// солдат. Бегом оружие чуть ниже, стволом немного вниз, и покачивается с плечами.
    fn hold_weapon(&self, p: &mut Pose, j: Joints, m: Motion, twist: f32) {
        let r = &self.rest;
        let (run, moving, ph) = (run_share(m.speed), (m.speed / 1.2).clamp(0.0, 1.0), m.phase);
        // Лёжа оружие не несут поперёк груди, и плечи не разворачивают: корпус лежит.
        let lying = m.prone.clamp(0.0, 1.0);
        let carry = m.carry * (1.0 - lying);
        // Целясь, корпус развёрнут левым плечом вперёд: приклад — у правого плеча, а левая
        // рука дотягивается до цевья. Неся оружие на бегу — прямо.
        let turn = Quat::from_rotation_y((-BLADE * (1.0 - carry) + twist) * (1.0 - lying));
        let pivot = p[j.mid];
        for i in [j.shoulder[0], j.shoulder[1], j.neck] {
            p[i] = pivot + turn * (p[i] - pivot);
        }
        let (rs, ls) = (p[j.shoulder[0]], p[j.shoulder[1]]);
        let sway = Vec3::Y * (2.0 * ph).cos() * 0.6 * run * moving;
        // Целится: рукоять — перед правой стороной груди (снаружи тела), ствол — по взгляду,
        // бегом чуть ниже.
        let aim_grip = rs + Vec3::new(10.8, -5.5, -4.1).lerp(PRONE_GRIP, lying) + sway;
        let aim_dir = Vec3::new(1.0, -0.08 * run * (1.0 - lying) + 0.03 * lying, 0.0).normalize();
        // Несёт на бегу (как в RWR): рукоять — у правого бока груди, ствол — поперёк груди
        // влево, почти горизонтально: с камеры сверху оружие лежит поперёк хода, а не
        // торчит вверх по экрану.
        let chest = (rs + ls) * 0.5;
        let carry_grip = chest + Vec3::new(6.0, -7.0, 4.0) + sway;
        let carry_dir = Vec3::new(0.25, 0.08, -0.96).normalize();
        let grip = aim_grip.lerp(carry_grip, carry);
        let dir = aim_dir.lerp(carry_dir, carry).normalize();
        // Цевьё — на линии ствола, докуда левая рука достаёт (5–10 вокселей от рукояти):
        // иначе ствол повело бы к руке.
        let reach_left = (r[j.shoulder[1]].distance(r[j.elbow[1]]) + r[j.elbow[1]].distance(r[j.hand[1]])) * 0.97;
        let to = grip - ls;
        let b = dir.dot(to);
        let along = (b * b - to.length_squared() + reach_left * reach_left).max(0.0).sqrt() - b;
        let left = grip + dir * along.clamp(5.0, 10.0);
        // Локти — вниз и наружу, не в грудь.
        for (side, hand, bend) in [(0, grip, Vec3::new(-0.2, -1.0, 0.7)), (1, left, Vec3::new(0.0, -1.0, -0.6))] {
            let shoulder = p[j.shoulder[side]];
            let upper = r[j.shoulder[side]].distance(r[j.elbow[side]]);
            let fore = r[j.elbow[side]].distance(r[j.hand[side]]);
            p[j.elbow[side]] = two_bone(shoulder, hand, upper, fore, bend);
            p[j.hand[side]] = shoulder + (hand - shoulder).clamp_length_max(upper + fore - 0.01);
        }
    }

    /// Шаг без записей движений: стопы по синусоиде, корпус пружинит и наклоняется.
    fn legs_procedural(&self, m: Motion, j: Joints) -> Pose {
        let mut p = self.rest.clone();
        let r = &self.rest;
        let moving = (m.speed / 1.2).clamp(0.0, 1.0);
        let run = run_share(m.speed);
        let dir = m.dir.normalize_or(Vec2::X);
        let along = Vec3::new(dir.x, 0.0, dir.y);
        let ph = m.phase;
        // Стопа ходит вперёд-назад: шагом размах — четверть круга (за полкруга опоры
        // стопа проходит столько же, сколько тело, и не скользит), бегом — 17 вокселей:
        // шаг длинный, но и часть круга обе ноги в воздухе.
        let reach = (12.0 + 5.0 * run) * (m.speed / 2.0).min(1.0);
        let lift = (3.0 + 5.0 * run) * moving;
        // Корпус: шагом выше всего над опорной ногой, бегом — в полёте. Бегом наклонён
        // вперёд по ходу и чуть поворачивается плечами навстречу шагу.
        let bob = ((0.5 - ph.sin().abs()) * 1.0 * (1.0 - run) + (ph.sin().abs() - 0.6) * 2.4 * run) * moving;
        let lean = (0.06 + 0.2 * run) * moving;
        let hip_y = (r[j.hip[0]].y + r[j.hip[1]].y) * 0.5;
        let upper = [j.head, j.neck, j.mid, j.shoulder[0], j.shoulder[1], j.elbow[0], j.elbow[1], j.hand[0], j.hand[1]];
        for i in upper {
            p[i] += along * (r[i].y - hip_y) * lean;
            p[i].y += bob;
        }
        for i in [j.hip[0], j.hip[1]] {
            p[i].y += bob;
        }

        // Ноги: стопа ходит по ходу движения по синусоиде и поднимается, пока идёт
        // вперёд; бегом пятка подлетает раньше — позади.
        for side in 0..2 {
            let ph = ph + side as f32 * std::f32::consts::PI;
            let hip = p[j.hip[side]];
            let mut foot = r[j.foot[side]] + along * (ph.sin() * reach);
            foot.y += (ph + 0.6 * run).cos().max(0.0) * lift;
            let thigh = r[j.hip[side]].distance(r[j.knee[side]]);
            let shin = r[j.knee[side]].distance(r[j.foot[side]]);
            p[j.foot[side]] = foot;
            p[j.knee[side]] = two_bone(hip, foot, thigh, shin, Vec3::X);
        }
        p
    }

    /// Матрицы палок, по одной на палку: из позы покоя в позу `pose` (метры, +X вперёд).
    ///
    /// Поворот палки = поворот корпуса + наклон палки кратчайшим путём. Поворот вокруг
    /// самой палки берётся от корпуса, а не от мира — иначе при падении или на шаге
    /// части проворачивались вокруг своей оси и тело «разъезжалось».
    ///
    /// Корпус — два: ноги и таз поворачиваются по бёдрам, а грудь, голова и руки — по
    /// плечам. Целясь, солдат разворачивает плечи (`BLADE`); раньше грудь и голова
    /// поворачивались по бёдрам и оставались прямо, а рукава уходили с плечами — на швах
    /// у плеч и шеи тело рвалось и вспучивалось.
    pub fn bones(&self, pose: &Pose, out: &mut impl FnMut(Mat4)) {
        let lower = self.body_frame(pose) * self.rest_body.transpose();
        let upper = self.upper_frame(pose) * self.rest_upper.transpose();
        for &(a, b) in &self.sticks {
            let body = if self.is_upper(b) { upper } else { lower };
            let turned = body * (self.rest[b] - self.rest[a]).normalize_or(Vec3::Y);
            let tilt = Quat::from_rotation_arc(turned, (pose[b] - pose[a]).normalize_or(turned));
            out(Mat4::from_translation(pose[a] * VOXEL)
                * Mat4::from_mat3(Mat3::from_quat(tilt) * body)
                * Mat4::from_translation(-self.rest[a] * VOXEL));
        }
    }

    /// Палка, которая кончается в точке `b`, — верх тела (грудь, голова, руки)?
    fn is_upper(&self, b: usize) -> bool {
        self.joints.is_some_and(|j| [j.neck, j.head, j.shoulder[0], j.shoulder[1], j.elbow[0], j.elbow[1], j.hand[0], j.hand[1]].contains(&b))
    }

    /// Поворот верха тела: «вверх» — от поясницы к шее, «вбок» — от плеча к плечу.
    fn upper_frame(&self, pose: &Pose) -> Mat3 {
        let Some(j) = self.joints else { return Mat3::IDENTITY };
        let up = (pose[j.neck] - pose[j.mid]).normalize_or(Vec3::Y);
        let across = pose[j.shoulder[1]] - pose[j.shoulder[0]];
        let side = (across - up * across.dot(up)).normalize_or(Vec3::Z);
        Mat3::from_cols(up.cross(side), up, side)
    }

    /// Поворот корпуса: «вверх» — от поясницы к шее, «вбок» — от бедра к бедру.
    fn body_frame(&self, pose: &Pose) -> Mat3 {
        let Some(j) = self.joints else { return Mat3::IDENTITY };
        let up = (pose[j.neck] - pose[j.mid]).normalize_or(Vec3::Y);
        let across = pose[j.hip[1]] - pose[j.hip[0]];
        let side = (across - up * across.dot(up)).normalize_or(Vec3::Z);
        Mat3::from_cols(up.cross(side), up, side)
    }

    /// Где и как держится оружие: начало координат оружия — в правой руке,
    /// ствол смотрит на левую руку. У RWR ствол направлен в −X, а у нас после
    /// смены осей это +Z оружия.
    pub fn weapon_frame(&self, pose: &Pose) -> Option<Mat4> {
        let j = self.joints?;
        let (rh, lh) = (pose[j.hand[0]], pose[j.hand[1]]);
        let aim = (lh - rh).normalize_or(Vec3::X);
        let z = aim;
        let y = (Vec3::Y - aim * aim.dot(Vec3::Y)).normalize_or(Vec3::Y);
        let x = y.cross(z);
        Some(Mat4::from_translation(rh * VOXEL) * Mat4::from_mat3(Mat3::from_cols(x, y, z)))
    }
}

fn side_sign(z: f32) -> f32 {
    if z >= 0.0 { 1.0 } else { -1.0 }
}

/// Обратная кинематика двух звеньев: где сустав (колено, локоть), если начало и конец
/// известны. `bend` — куда сгибаться.
fn two_bone(root: Vec3, end: Vec3, a: f32, b: f32, bend: Vec3) -> Vec3 {
    let to = end - root;
    let len = to.length().clamp(0.001, a + b - 0.001);
    let dir = to.normalize_or(Vec3::NEG_Y);
    let along = (a * a - b * b + len * len) / (2.0 * len);
    let h = (a * a - along * along).max(0.0).sqrt();
    let n = (bend - dir * bend.dot(dir)).normalize_or(Vec3::X);
    root + dir * along + n * h
}

// --- Сборка мешей ------------------------------------------------------------------

#[derive(Default)]
struct Builder {
    meshes: Vec<CharMesh>,
}

impl Builder {
    fn character(&mut self, name: &str, m: &RwrModel) -> CharModel {
        let rest: Vec<Vec3> = m.particles.iter().map(|p| p.pos).collect();
        // Все воксели модели: грань части, закрытая вокселем соседней части, не нужна.
        let solid: HashSet<IVec3> = m.voxels.iter().map(|v| v.0).collect();
        let has_voxels: Vec<bool> = m.bindings.iter().map(|b| !b.is_empty()).collect();
        // Соседняя палка для плавного сгиба: из палок с вокселями, что сходятся с этой
        // в суставе, — ближайшая к точке. Возвращает её номер и расстояние до сустава.
        let partner = |s: usize, p: Vec3| -> (u8, u8) {
            let (a, b) = m.sticks[s];
            let mut best: Option<(f32, usize, usize)> = None;
            for joint in [a, b] {
                for (q, &(qa, qb)) in m.sticks.iter().enumerate() {
                    if q == s || !has_voxels[q] || (qa != joint && qb != joint) {
                        continue;
                    }
                    let d = segment_distance(p, rest[qa], rest[qb]);
                    if best.is_none_or(|(bd, ..)| d < bd) {
                        best = Some((d, q, joint));
                    }
                }
            }
            match best {
                Some((_, q, joint)) => (q as u8, (p.distance(rest[joint]) * 4.0).min(255.0) as u8),
                None => (s as u8, 255),
            }
        };
        let mut mesh = CharMesh::default();
        for s in 0..m.sticks.len() {
            let voxels: Vec<_> = m.bindings[s].iter().map(|&i| m.voxels[i]).collect();
            if !voxels.is_empty() {
                add_part(&mut mesh, &voxels, s as u8, Some(&solid), |p| partner(s, p));
            }
        }
        self.meshes.push(mesh);
        let mesh = self.meshes.len() - 1;
        let find = |n: &str| m.particle(n);
        let pair = |r: &str, l: &str| Some([find(r)?, find(l)?]);
        let joints = (|| {
            Some(Joints {
                head: find("head")?,
                neck: find("neck")?,
                mid: find("midspine")?,
                shoulder: pair("rightshoulder", "leftshoulder")?,
                elbow: pair("rightelbow", "leftelbow")?,
                hand: pair("righthand", "lefthand")?,
                hip: pair("righthip", "lefthip")?,
                knee: pair("rightknee", "leftknee")?,
                foot: pair("rightfoot", "leftfoot")?,
            })
        })();
        let inv_mass: Vec<f32> = m.particles.iter().map(|p| p.inv_mass).collect();
        let torso = joints.map(|j| ([j.neck, j.mid, j.shoulder[0], j.shoulder[1], j.hip[0], j.hip[1]], j.head));
        let rig = Rig::new(&rest, &m.sticks, &inv_mass, torso);
        let mut model = CharModel { name: name.to_owned(), rest, sticks: m.sticks.clone(), mesh, joints, rest_body: Mat3::IDENTITY, rest_upper: Mat3::IDENTITY, rig, clips: HashMap::new() };
        model.rest_body = model.body_frame(&model.rest);
        model.rest_upper = model.upper_frame(&model.rest);
        model
    }

    /// Предмет без скелета (оружие): одна палка, её матрица — положение предмета.
    /// `grip` — где у модели рукоять, в осях файла RWR: она встанет в начало координат
    /// (в правую руку). Воксели уже в наших осях: (x, y, z) RWR — это (z, y, −x).
    fn prop(&mut self, m: &RwrModel, grip: [f32; 3]) -> usize {
        let mut mesh = CharMesh::default();
        let [x, y, z] = grip.map(|v| v.round() as i32);
        let shift = -IVec3::new(z, y, -x);
        let voxels: Vec<_> = m.voxels.iter().map(|&(p, c)| (p + shift, c)).collect();
        add_part(&mut mesh, &voxels, 0, None, |_| (0, 255));
        self.meshes.push(mesh);
        self.meshes.len() - 1
    }
}

/// Расстояние от точки до отрезка.
fn segment_distance(p: Vec3, a: Vec3, b: Vec3) -> f32 {
    let ab = b - a;
    let t = ((p - a).dot(ab) / ab.length_squared().max(1e-6)).clamp(0.0, 1.0);
    p.distance(a + ab * t)
}

/// Воксели одной палки в меш модели: грани для теней склеиваются по форме, а каждый
/// видимый воксель становится точкой со своим цветом. `partner` — соседняя палка
/// и расстояние до сустава для точки (в вокселях позы покоя).
fn add_part(
    mesh: &mut CharMesh,
    voxels: &[(IVec3, [u8; 4])],
    bone: u8,
    whole: Option<&HashSet<IVec3>>,
    partner: impl Fn(Vec3) -> (u8, u8),
) {
    let min = voxels.iter().fold(IVec3::MAX, |a, v| a.min(v.0));
    let max = voxels.iter().fold(IVec3::MIN, |a, v| a.max(v.0));
    let size = max - min + IVec3::ONE;
    let mut filled_cells = vec![false; (size.x * size.y * size.z) as usize];
    let idx = |p: IVec3| ((p.z * size.y + p.y) * size.x + p.x) as usize;
    for &(c, _) in voxels {
        filled_cells[idx(c - min)] = true;
    }
    let filled = |p: IVec3| p.cmpge(IVec3::ZERO).all() && p.cmplt(size).all() && filled_cells[idx(p)];
    let solid = |p: IVec3| filled(p) || whole.is_some_and(|w| w.contains(&(p + min)));
    for (corners, _) in greedy_shape(size, filled, solid) {
        let base = mesh.vertices.len() as u32;
        for c in corners {
            mesh.vertices.push(CharVertex { pos: ((c + min).as_vec3() * VOXEL).to_array(), bone: bone as u32 });
        }
        mesh.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }
    const SIDES: [IVec3; 6] = [IVec3::X, IVec3::NEG_X, IVec3::Y, IVec3::NEG_Y, IVec3::Z, IVec3::NEG_Z];
    // Воксель, со всех сторон закрытый другими, не виден.
    let visible: Vec<_> = voxels.iter().filter(|(c, _)| !SIDES.iter().all(|&d| solid(*c - min + d))).collect();
    for (lod, out) in mesh.lods.iter_mut().enumerate() {
        // Кубик k×k×k: цвет — средний, точка — в центре кубика.
        let k = 1 << lod;
        let mut cells: HashMap<IVec3, ([u32; 3], u32)> = HashMap::new();
        for &&(c, rgba) in &visible {
            let e = cells.entry(c.div_euclid(IVec3::splat(k))).or_default();
            for (sum, c) in e.0.iter_mut().zip(rgba) {
                *sum += c as u32;
            }
            e.1 += 1;
        }
        let mut cells: Vec<_> = cells.into_iter().collect();
        cells.sort_by_key(|(c, _)| (c.z, c.y, c.x));
        for (cell, (sum, n)) in cells {
            let center = (cell * k).as_vec3() + Vec3::splat(k as f32 * 0.5);
            let [r, g, b] = sum.map(|v| (v / n) as u8);
            let (other, dist) = partner(center);
            let base = out.points.len() as u32;
            for corner in 0..4 {
                out.points.push(PointVertex {
                    pos: (center * VOXEL).to_array(),
                    color: [r, g, b, corner | (lod as u8) << 2],
                    bones: [bone, other, dist, 0],
                });
            }
            out.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
}

/// Жадная склейка граней по форме: возвращает четырёхугольники (углы против часовой,
/// если смотреть со стороны нормали) и нормаль.
fn greedy_shape(size: IVec3, filled: impl Fn(IVec3) -> bool, solid: impl Fn(IVec3) -> bool) -> Vec<([IVec3; 4], [i8; 3])> {
    let size = size.to_array();
    let mut quads = Vec::new();
    for d in 0..3 {
        let (u, v) = ((d + 1) % 3, (d + 2) % 3);
        let mut mask = vec![0i8; (size[u] * size[v]) as usize];
        for slice in -1..size[d] {
            for j in 0..size[v] {
                for i in 0..size[u] {
                    let mut p = [0; 3];
                    p[d] = slice;
                    p[u] = i;
                    p[v] = j;
                    let mut q = p;
                    q[d] += 1;
                    let (p, q) = (IVec3::from_array(p), IVec3::from_array(q));
                    // Грань есть, если с одной стороны воксель части, а с другой — пусто во всей модели.
                    mask[(j * size[u] + i) as usize] = if filled(p) && !solid(q) {
                        1
                    } else if filled(q) && !solid(p) {
                        -1
                    } else {
                        0
                    };
                }
            }
            for j in 0..size[v] {
                let mut i = 0;
                while i < size[u] {
                    let c = mask[(j * size[u] + i) as usize];
                    if c == 0 {
                        i += 1;
                        continue;
                    }
                    let mut w = 1;
                    while i + w < size[u] && mask[(j * size[u] + i + w) as usize] == c {
                        w += 1;
                    }
                    let mut h = 1;
                    'grow: while j + h < size[v] {
                        for k in 0..w {
                            if mask[((j + h) * size[u] + i + k) as usize] != c {
                                break 'grow;
                            }
                        }
                        h += 1;
                    }
                    let mut base = [0; 3];
                    base[d] = slice + 1;
                    base[u] = i;
                    base[v] = j;
                    let (mut du, mut dv) = ([0; 3], [0; 3]);
                    du[u] = w;
                    dv[v] = h;
                    let p0 = IVec3::from_array(base);
                    let (du, dv) = (IVec3::from_array(du), IVec3::from_array(dv));
                    let mut normal = [0i8; 3];
                    normal[d] = 127 * c;
                    let corners = if c > 0 { [p0, p0 + du, p0 + du + dv, p0 + dv] } else { [p0, p0 + dv, p0 + du + dv, p0 + du] };
                    quads.push((corners, normal));
                    for l in 0..h {
                        for k in 0..w {
                            mask[((j + l) * size[u] + i + k) as usize] = 0;
                        }
                    }
                    i += w;
                }
            }
        }
    }
    quads
}

#[cfg(test)]
mod tests {
    use super::*;

    fn data() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")
    }

    #[test]
    fn loads_factions_and_meshes_are_small() {
        let set = CharacterSet::load(&data()).unwrap();
        assert!(!set.models.is_empty() && !set.weapons.is_empty());
        let m = &set.models[0];
        assert_eq!(m.sticks.len(), 17);
        let mesh = &set.meshes[m.mesh];
        let quads = mesh.indices.len() / 6;
        let points = |lod: usize| mesh.lods[lod].points.len() / 4;
        // У суставов у точек есть соседняя палка.
        assert!(mesh.lods[0].points.iter().any(|p| p.bones[0] != p.bones[1] && p.bones[2] < 16));
        // Склейка по форме: сотни граней, а не тысячи.
        assert!(quads < 3500, "граней у солдата: {quads}");
        assert!(points(0) > 500 && points(0) < 4000, "точек у солдата: {}", points(0));
        // Каждый следующий уровень детализации заметно легче.
        assert!(points(1) * 2 < points(0) && points(2) * 2 < points(1));
        println!("граней у солдата: {quads}, точек: {} / {} / {}", points(0), points(1), points(2));
        assert!(set.pick(0, 5).is_some() && set.pick(1, 5).is_some());
    }

    /// Правая ладонь (начало модели оружия) лежит на оружии, а не под ним: иначе оружие
    /// висит над руками (у ПП-2000 и ГМ-94 рукоять — не в начале модели, см. `grip`).
    #[test]
    fn every_weapon_is_held_by_its_grip() {
        let set = CharacterSet::load(&data()).unwrap();
        for w in &set.weapons {
            let nearest = set.meshes[w.mesh].lods[0].points.iter().map(|p| (Vec3::from_array(p.pos) / VOXEL).length()).fold(f32::MAX, f32::min);
            println!("{}: до оружия от ладони {nearest:.1} вокселя", w.name);
            assert!(nearest < 2.5, "{}: ладонь в {nearest:.1} вокселя от оружия — висит", w.name);
        }
    }

    #[test]
    fn rest_pose_keeps_parts_in_place() {
        let set = CharacterSet::load(&data()).unwrap();
        let m = &set.models[0];
        let mut moved = 0.0f32;
        m.bones(&m.rest, &mut |mat| {
            // В позе покоя каждая палка остаётся на месте: матрица единичная.
            moved = moved.max((mat - Mat4::IDENTITY).abs().to_cols_array().iter().copied().fold(0.0, f32::max));
        });
        assert!(moved < 1e-4, "{moved}");
    }

    /// Матрицы палок, по порядку.
    fn part_matrices(m: &CharModel, pose: &Pose) -> Vec<Mat4> {
        let mut out = Vec::new();
        m.bones(pose, &mut |mat| out.push(mat));
        out
    }

    #[test]
    fn stances_crouch_and_lie_low_and_keep_weapon() {
        let set = CharacterSet::load(&data()).unwrap();
        for m in &set.models {
            let j = m.joints.unwrap();
            let standing = m.pose(Motion::default());
            let crouched = m.pose(Motion { crouch: 1.0, ..Default::default() });
            let lying = m.pose(Motion { prone: 1.0, ..Default::default() });
            let top = |p: &Pose| p.iter().map(|v| v.y).fold(f32::MIN, f32::max);
            // Присел — голова на ~1,25 м против ~1,75 стоя (у RWR — 1,15), лёжа — низко.
            assert!(top(&crouched) < top(&standing) * 0.8 && top(&crouched) > top(&standing) * 0.55, "{}: присел ({} против {})", m.name, top(&crouched), top(&standing));
            assert!(top(&lying) < top(&standing) * 0.4, "{}: лежит низко ({} против {})", m.name, top(&lying), top(&standing));
            // Присел на правое колено: оно у земли, левая стопа впереди.
            assert!(crouched[j.knee[0]].y < 4.0 && crouched[j.foot[1]].x > crouched[j.knee[0]].x + 5.0, "{}: колено {:?}", m.name, crouched[j.knee[0]]);
            // Оружие в руках и смотрит вперёд; лёжа — вдоль земли.
            for (p, what) in [(&crouched, "присел"), (&lying, "лежит")] {
                let aim = m.weapon_frame(p).expect("оружие в руках").z_axis.truncate();
                assert!(aim.x > 0.9 && aim.y.abs() < 0.3, "{}, {what}: ствол вперёд ({aim:?})", m.name);
            }
            // Лёжа, как у RWR: голова впереди, тело — назад от груди.
            assert!(lying[j.head].x > 3.0 && lying[j.foot[0]].x < -25.0, "{}: голова {:?}, стопа {:?}", m.name, lying[j.head], lying[j.foot[0]]);
            // Ложится: посередине — между приседом и «лежит», без рывков по ходу клипа.
            let mid = m.pose(Motion { prone: 0.5, ..Default::default() });
            assert!(top(&mid) < top(&standing) && top(&mid) > top(&lying));
            let mut last = m.pose(Motion { prone: 0.0001, ..Default::default() });
            for k in 1..=50 {
                let now = m.pose(Motion { prone: k as f32 / 50.0, ..Default::default() });
                let jump = now.iter().zip(&last).map(|(a, b)| a.distance(*b)).fold(0.0, f32::max);
                assert!(jump < 4.0, "{}: рывок {jump:.1} вокселя на {k}/50, пока ложится", m.name);
                last = now;
            }
        }
    }

    #[test]
    fn walking_parts_turn_smoothly() {
        let set = CharacterSet::load(&data()).unwrap();
        for m in &set.models {
            let mut last: Option<Vec<Mat4>> = None;
            for step in 0..400 {
                let pose = m.pose(Motion { speed: 6.0 + (step % 7) as f32 * 0.3, phase: step as f32 * 0.05, ..Default::default() });
                let now = part_matrices(m, &pose);
                if let Some(prev) = &last {
                    for (a, b) in prev.iter().zip(&now) {
                        let jump = (Mat3::from_mat4(*a) - Mat3::from_mat4(*b)).abs().to_cols_array().iter().copied().fold(0.0, f32::max);
                        assert!(jump < 0.3, "{}: часть резко повернулась за шаг ({jump})", m.name);
                    }
                }
                last = Some(now);
            }
        }
    }

    #[test]
    fn walking_feet_stay_near_ground_and_legs_keep_length() {
        let set = CharacterSet::load(&data()).unwrap();
        let m = &set.models[0];
        let j = m.joints.unwrap();
        for step in 0..16 {
            let pose = m.pose(Motion { speed: 6.0, phase: step as f32 * 0.4, ..Default::default() });
            for side in 0..2 {
                let thigh = m.rest[j.hip[side]].distance(m.rest[j.knee[side]]);
                let now = pose[j.hip[side]].distance(pose[j.knee[side]]);
                assert!((thigh - now).abs() < 0.05, "бедро {thigh} → {now}");
                // Бегом пятка подлетает выше, чем шагом, но не выше колена другой ноги.
                assert!(pose[j.foot[side]].y < 10.0);
            }
        }
        assert!(m.weapon_frame(&m.pose(Motion::default())).is_some());
    }

    #[test]
    fn rifle_points_where_the_soldier_looks() {
        let set = CharacterSet::load(&data()).unwrap();
        for m in &set.models {
            for speed in [0.0, 2.2, 6.0] {
                for step in 0..12 {
                    let pose = m.pose(Motion { speed, phase: step as f32 * 0.5, ..Default::default() });
                    let aim = m.weapon_frame(&pose).unwrap().z_axis.truncate();
                    // Вперёд, не в сторону; бегом — чуть вниз и покачивается с плечами.
                    let side = if speed > 3.0 { 0.2 } else { 0.1 };
                    assert!(aim.z.abs() < side && aim.x > 0.95, "{}, {speed} м/с: ствол смотрит не вперёд ({aim:?})", m.name);
                }
            }
        }
    }

    #[test]
    fn hands_stay_outside_the_chest_and_running_carries_the_rifle_across() {
        let set = CharacterSet::load(&data()).unwrap();
        for m in &set.models {
            let j = m.joints.unwrap();
            for speed in [0.0, 2.2, 6.0] {
                let p = m.pose(Motion { speed, phase: 1.0, ..Default::default() });
                // Целится: рукоять — впереди груди (не внутри), левая рука — ещё дальше.
                let chest = (p[j.shoulder[0]] + p[j.shoulder[1]]) * 0.5;
                assert!(p[j.hand[0]].x > chest.x + 4.0, "{}, {speed} м/с: правая кисть в груди ({:?})", m.name, p[j.hand[0]]);
                assert!(p[j.hand[1]].x > p[j.hand[0]].x + 4.0, "{}: левая кисть не на цевье", m.name);
            }
            // Несёт на бегу: ствол поперёк хода, влево, почти горизонтально.
            let p = m.pose(Motion { speed: 6.0, phase: 1.0, carry: 1.0, ..Default::default() });
            let aim = m.weapon_frame(&p).unwrap().z_axis.truncate();
            assert!(aim.z < -0.85 && aim.y.abs() < 0.2, "{}: на бегу ствол не поперёк ({aim:?})", m.name);
        }
    }

    #[test]
    fn running_strides_longer_and_legs_follow_the_move_direction() {
        let set = CharacterSet::load(&data()).unwrap();
        let m = &set.models[0];
        let j = m.joints.unwrap();
        // Размах стоп по ходу: бегом шаг длиннее, круг шага — длиннее по пути.
        let spread = |speed: f32, dir: Vec2| {
            (0..32)
                .map(|i| {
                    let pose = m.pose(Motion { speed, phase: i as f32 * std::f32::consts::TAU / 32.0, dir, carry: 0.0, ..Default::default() });
                    let f = pose[j.foot[0]] - m.rest[j.foot[0]];
                    Vec2::new(f.x, f.z)
                })
                .collect::<Vec<_>>()
        };
        let reach = |v: &[Vec2]| v.iter().map(|f| f.length()).fold(0.0, f32::max);
        assert!(reach(&spread(6.0, Vec2::X)) > reach(&spread(2.2, Vec2::X)));
        assert!(set.stride_rate(6.0) < set.stride_rate(2.2), "бегом круг шага длиннее");
        // Идёт вправо — стопы ходят вправо-влево, а не вперёд-назад.
        let side = spread(2.2, Vec2::Y);
        let (along_x, along_z) = side.iter().fold((0.0f32, 0.0f32), |(x, z), f| (x.max(f.x.abs()), z.max(f.y.abs())));
        assert!(along_z > 8.0 && along_z > along_x * 2.5, "вбок: размах вбок {along_z:.1}, вперёд {along_x:.1}");
        // Клипы стандарта анимаций: у моделей есть шаг и бег.
        assert!(set.models.iter().all(|m| m.clips.contains_key("walking") && m.clips.contains_key("running")), "клипы data/animations не подхватились");
    }
}
