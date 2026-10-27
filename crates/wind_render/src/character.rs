//! Свои солдаты: модели RWR со скелетом, процедурная поза, оружие, тела.
//!
//! У каждой палки скелета свои воксели и своя матрица; матрицы всех палок солдата
//! уходят в видеокарту, и модель рисуется одним вызовом на всех солдат (как у RWR).
//! Воксель у сустава смешивает положение между своей палкой и соседней — сгиб плавный.
//! Как у RWR, солдат рисуется облаком точек: каждый воксель — квадратик постоянного
//! размера на экране (см. заметку «Как устроена картинка RWR»). Грани, склеенные
//! по форме, остались только для теней.

use crate::rwr::{self, RwrModel, VOXEL};
use wind_sim::arsenal::Arsenal;
use bytemuck::{Pod, Zeroable};
use glam::{IVec3, Mat3, Mat4, Quat, Vec3};
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
    /// Поворот корпуса в позе покоя (см. `body_frame`).
    rest_body: Mat3,
}

pub struct WeaponModel {
    pub name: String,
    pub mesh: usize,
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
                        let mesh = builder.prop(&read(path)?);
                        weapons.push(WeaponModel { name: path.clone(), mesh });
                        set_weapons.push((path.clone(), weapons.len() - 1));
                        weapons.len() - 1
                    }
                };
                teams[team].push((model, weapon));
            }
        }
        Ok(CharacterSet { meshes: builder.meshes, models, weapons, teams, team_names: arsenal.team_names.clone(), arsenal })
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
    /// Фаза шага, рад.
    pub phase: f32,
}

impl CharModel {
    /// Живой солдат: ноги шагают, руки держат оружие.
    pub fn pose(&self, m: Motion) -> Pose {
        let mut p = self.rest.clone();
        let Some(j) = self.joints else { return p };
        let r = &self.rest;
        let k = (m.speed / 4.5).clamp(0.0, 1.3);
        let stride = 9.0 * k;
        let lift = 4.0 * k;

        // Корпус: чуть пружинит на каждом шаге и наклоняется вперёд на бегу.
        let bob = -(m.phase * 2.0).cos().abs() * 0.8 * k + 0.4 * k;
        let hip_y = (r[j.hip[0]].y + r[j.hip[1]].y) * 0.5;
        let lean = 0.12 * k;
        for i in [j.head, j.neck, j.mid, j.shoulder[0], j.shoulder[1], j.elbow[0], j.elbow[1], j.hand[0], j.hand[1]] {
            p[i].x += (r[i].y - hip_y) * lean;
            p[i].y += bob;
        }
        for i in [j.hip[0], j.hip[1]] {
            p[i].y += bob;
        }

        // Ноги: стопа ходит вперёд-назад по синусоиде и поднимается, пока идёт вперёд.
        for side in 0..2 {
            let ph = m.phase + side as f32 * std::f32::consts::PI;
            let hip = p[j.hip[side]];
            let mut foot = r[j.foot[side]];
            foot.x += ph.sin() * stride;
            foot.y += ph.cos().max(0.0) * lift;
            let thigh = r[j.hip[side]].distance(r[j.knee[side]]);
            let shin = r[j.knee[side]].distance(r[j.foot[side]]);
            p[j.foot[side]] = foot;
            p[j.knee[side]] = two_bone(hip, foot, thigh, shin, Vec3::X);
        }

        // Руки: правая у приклада, левая под цевьём. Локти — вниз и чуть наружу.
        let shoulder_y = (p[j.shoulder[0]].y + p[j.shoulder[1]].y) * 0.5;
        let chest_x = (p[j.shoulder[0]].x + p[j.shoulder[1]].x) * 0.5;
        let right_hand = Vec3::new(chest_x + 7.0, shoulder_y - 6.0, 2.5);
        let left_hand = right_hand + Vec3::new(12.0, 0.6, -1.5);
        for (side, hand, out) in [(0, right_hand, 1.0), (1, left_hand, -1.0)] {
            let shoulder = p[j.shoulder[side]];
            let upper = r[j.shoulder[side]].distance(r[j.elbow[side]]);
            let fore = r[j.elbow[side]].distance(r[j.hand[side]]);
            let bend = Vec3::new(0.0, -1.0, 0.6 * out * side_sign(r[j.shoulder[side]].z));
            p[j.elbow[side]] = two_bone(shoulder, hand, upper, fore, bend);
            p[j.hand[side]] = shoulder + (hand - shoulder).clamp_length_max(upper + fore - 0.01);
        }
        p
    }

    /// Лёжа: поза `pose`, опрокинутая лицом вниз на долю `t` (0 — стоит, 1 — лежит),
    /// голова — туда, куда смотрит солдат. Ноги вытягиваются, а руки уходят «вверх» —
    /// после поворота это вперёд, перед головой: оружие лежит вдоль земли и смотрит
    /// на врага. Так залёгший не похож на убитого.
    pub fn lie(&self, pose: &Pose, t: f32) -> Pose {
        let t = t.clamp(0.0, 1.0);
        let t = t * t * (3.0 - 2.0 * t);
        let mut p = pose.clone();
        if let Some(j) = self.joints {
            let r = &self.rest;
            for i in [j.knee[0], j.knee[1], j.foot[0], j.foot[1]] {
                p[i] = p[i].lerp(r[i], t);
            }
            let shoulder_y = (p[j.shoulder[0]].y + p[j.shoulder[1]].y) * 0.5;
            let chest_x = (p[j.shoulder[0]].x + p[j.shoulder[1]].x) * 0.5;
            let right_hand = Vec3::new(chest_x + 1.0, shoulder_y + 4.0, 2.5);
            let left_hand = right_hand + Vec3::new(0.0, 12.0, -1.5);
            for (side, hand, out) in [(0, right_hand, 1.0), (1, left_hand, -1.0)] {
                let shoulder = p[j.shoulder[side]];
                let hand = p[j.hand[side]].lerp(hand, t);
                let upper = r[j.shoulder[side]].distance(r[j.elbow[side]]);
                let fore = r[j.elbow[side]].distance(r[j.hand[side]]);
                // Локти — к груди: после поворота это вниз, на землю.
                let bend = Vec3::new(1.0, 0.0, 0.6 * out * side_sign(r[j.shoulder[side]].z));
                p[j.elbow[side]] = two_bone(shoulder, hand, upper, fore, bend);
                p[j.hand[side]] = shoulder + (hand - shoulder).clamp_length_max(upper + fore - 0.01);
            }
        }
        let rot = Mat3::from_rotation_z(-std::f32::consts::FRAC_PI_2 * t);
        let lift = 3.0 * t;
        p.iter().map(|&v| rot * v + Vec3::Y * lift).collect()
    }

    /// Убитый: падает назад (или вперёд) за полсекунды и лежит, раскинув руки.
    pub fn corpse(&self, age: f32, forward: bool) -> Pose {
        let t = (age / 0.45).clamp(0.0, 1.0);
        let t = t * t * (3.0 - 2.0 * t);
        let angle = if forward { -1.0 } else { 1.0 } * std::f32::consts::FRAC_PI_2 * t;
        let rot = Mat3::from_rotation_z(angle);
        let lift = 3.0 * t;
        self.rest.iter().map(|&v| rot * v + Vec3::Y * lift).collect()
    }

    /// Матрицы палок, по одной на палку: из позы покоя в позу `pose` (метры, +X вперёд).
    ///
    /// Поворот палки = поворот корпуса + наклон палки кратчайшим путём. Поворот вокруг
    /// самой палки берётся от корпуса, а не от мира — иначе при падении или на шаге
    /// части проворачивались вокруг своей оси и тело «разъезжалось».
    pub fn bones(&self, pose: &Pose, out: &mut impl FnMut(Mat4)) {
        let body = self.body_frame(pose) * self.rest_body.transpose();
        for &(a, b) in &self.sticks {
            let turned = body * (self.rest[b] - self.rest[a]).normalize_or(Vec3::Y);
            let tilt = Quat::from_rotation_arc(turned, (pose[b] - pose[a]).normalize_or(turned));
            out(Mat4::from_translation(pose[a] * VOXEL)
                * Mat4::from_mat3(Mat3::from_quat(tilt) * body)
                * Mat4::from_translation(-self.rest[a] * VOXEL));
        }
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
        let mut model = CharModel { name: name.to_owned(), rest, sticks: m.sticks.clone(), mesh, joints, rest_body: Mat3::IDENTITY };
        model.rest_body = model.body_frame(&model.rest);
        model
    }

    /// Предмет без скелета (оружие): одна палка, её матрица — положение предмета.
    fn prop(&mut self, m: &RwrModel) -> usize {
        let mut mesh = CharMesh::default();
        add_part(&mut mesh, &m.voxels, 0, None, |_| (0, 255));
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
    fn corpse_falls_as_one_piece() {
        let set = CharacterSet::load(&data()).unwrap();
        for m in &set.models {
            for forward in [false, true] {
                let pose = m.corpse(10.0, forward);
                let angle = if forward { -1.0 } else { 1.0 } * std::f32::consts::FRAC_PI_2;
                let rot = Mat3::from_rotation_z(angle);
                for mat in part_matrices(m, &pose) {
                    let diff = (Mat3::from_mat4(mat) - rot).abs().to_cols_array().iter().copied().fold(0.0, f32::max);
                    assert!(diff < 1e-3, "{}: часть повёрнута не как всё тело ({diff})", m.name);
                }
            }
        }
    }

    #[test]
    fn prone_soldier_lies_low_and_keeps_weapon() {
        let set = CharacterSet::load(&data()).unwrap();
        for m in &set.models {
            let standing = m.pose(Motion::default());
            let lying = m.lie(&standing, 1.0);
            let top = |p: &Pose| p.iter().map(|v| v.y).fold(f32::MIN, f32::max);
            assert!(top(&lying) < top(&standing) * 0.4, "{}: лежит низко ({} против {})", m.name, top(&lying), top(&standing));
            // Оружие в руках, вдоль земли и вперёд.
            let grip = m.weapon_frame(&lying).expect("оружие в руках");
            let aim = grip.z_axis.truncate();
            assert!(aim.x > 0.9 && aim.y.abs() < 0.3, "{}: ствол вперёд вдоль земли ({aim:?})", m.name);
            // Голова — вперёд, туда, куда смотрит солдат (ось x модели).
            let head = lying.iter().copied().fold(Vec3::splat(f32::MIN), Vec3::max);
            assert!(head.x > 10.0, "{}: голова впереди ({head:?})", m.name);
            // Наполовину — как раз посередине между «стоит» и «лежит», без рывков.
            assert!(top(&m.lie(&standing, 0.5)) < top(&standing) && top(&m.lie(&standing, 0.5)) > top(&lying));
        }
    }

    #[test]
    fn walking_parts_turn_smoothly() {
        let set = CharacterSet::load(&data()).unwrap();
        for m in &set.models {
            let mut last: Option<Vec<Mat4>> = None;
            for step in 0..400 {
                let pose = m.pose(Motion { speed: 4.5 + (step % 7) as f32 * 0.3, phase: step as f32 * 0.05 });
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
            let pose = m.pose(Motion { speed: 4.5, phase: step as f32 * 0.4 });
            for side in 0..2 {
                let thigh = m.rest[j.hip[side]].distance(m.rest[j.knee[side]]);
                let now = pose[j.hip[side]].distance(pose[j.knee[side]]);
                assert!((thigh - now).abs() < 0.05, "бедро {thigh} → {now}");
                assert!(pose[j.foot[side]].y < 6.0);
            }
        }
        assert!(m.weapon_frame(&m.pose(Motion::default())).is_some());
    }
}
