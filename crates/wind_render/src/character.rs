//! Свои солдаты: модели RWR со скелетом, процедурная поза, оружие, тела.
//!
//! Каждая палка скелета — жёсткая часть со своими вокселями. Грани части склеиваются
//! только по форме, а цвет каждого вокселя лежит в общей 3D-текстуре (атласе):
//! у камуфляжа почти каждый воксель своего цвета, и склейка по цвету не помогла бы.

use crate::rwr::{self, RwrModel, VOXEL};
use bytemuck::{Pod, Zeroable};
use glam::{IVec3, Mat3, Mat4, Vec3};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::Path;

/// Вершина части солдата: позиция (м, относительно точки крепления), нормаль
/// и координата угла вокселя в атласе — по ней шейдер берёт цвет.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct CharVertex {
    pub pos: [f32; 3],
    pub normal: [i8; 4],
    pub atlas: [u16; 4],
}

#[derive(Clone, Debug, Default)]
pub struct CharMesh {
    pub vertices: Vec<CharVertex>,
    pub indices: Vec<u32>,
}

/// Жёсткая часть: палка скелета и её меш.
pub struct Part {
    pub stick: (usize, usize),
    pub mesh: usize,
    /// Точка крепления (начало палки) в позе покоя, воксели.
    pub rest_origin: Vec3,
    pub rest_frame: Mat3,
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
    pub parts: Vec<Part>,
    joints: Option<Joints>,
}

pub struct WeaponModel {
    pub name: String,
    pub mesh: usize,
}

#[derive(Deserialize)]
struct FactionsFile {
    teams: Vec<TeamFile>,
}

#[derive(Deserialize)]
struct TeamFile {
    name: String,
    soldiers: Vec<SoldierFile>,
}

#[derive(Deserialize)]
struct SoldierFile {
    model: String,
    weapon: String,
    #[serde(default = "one")]
    weight: u32,
}

fn one() -> u32 {
    1
}

/// Всё про своих солдат: меши, атлас цветов, модели, кто в какой команде.
pub struct CharacterSet {
    pub meshes: Vec<CharMesh>,
    /// Атлас цветов: размер и RGBA по вокселям (x быстрее всего, потом y, потом z).
    pub atlas_size: [u32; 3],
    pub atlas: Vec<u8>,
    pub models: Vec<CharModel>,
    pub weapons: Vec<WeaponModel>,
    /// Для каждой команды: (модель, оружие, вес).
    pub teams: [Vec<(usize, usize, u32)>; 2],
    pub team_names: [String; 2],
}

/// Поза — положения 15 точек скелета в пространстве модели (воксели, +X вперёд).
pub type Pose = Vec<Vec3>;

impl CharacterSet {
    /// Читает `data/factions.ron` и все упомянутые там модели.
    pub fn load(data_dir: &Path) -> Result<Self, String> {
        let path = data_dir.join("factions.ron");
        let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let file: FactionsFile = ron::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        if file.teams.len() != 2 {
            return Err(format!("{}: нужно ровно две команды", path.display()));
        }
        let mut builder = Builder::default();
        let mut set_models: Vec<(String, usize)> = Vec::new();
        let mut set_weapons: Vec<(String, usize)> = Vec::new();
        let mut models = Vec::new();
        let mut weapons = Vec::new();
        let mut teams: [Vec<(usize, usize, u32)>; 2] = Default::default();
        let read = |name: &str| -> Result<RwrModel, String> {
            let p = data_dir.join("models").join(format!("{name}.xml"));
            let text = std::fs::read_to_string(&p).map_err(|e| format!("{}: {e}", p.display()))?;
            rwr::parse(&text).map_err(|e| format!("{}: {e}", p.display()))
        };
        for (team, t) in file.teams.iter().enumerate() {
            for s in &t.soldiers {
                let model = match set_models.iter().find(|(n, _)| *n == s.model) {
                    Some(&(_, i)) => i,
                    None => {
                        models.push(builder.character(&s.model, &read(&s.model)?));
                        set_models.push((s.model.clone(), models.len() - 1));
                        models.len() - 1
                    }
                };
                let weapon = match set_weapons.iter().find(|(n, _)| *n == s.weapon) {
                    Some(&(_, i)) => i,
                    None => {
                        let mesh = builder.prop(&read(&s.weapon)?);
                        weapons.push(WeaponModel { name: s.weapon.clone(), mesh });
                        set_weapons.push((s.weapon.clone(), weapons.len() - 1));
                        weapons.len() - 1
                    }
                };
                teams[team].push((model, weapon, s.weight.max(1)));
            }
        }
        let (atlas_size, atlas) = builder.finish_atlas();
        Ok(CharacterSet {
            meshes: builder.meshes,
            atlas_size,
            atlas,
            models,
            weapons,
            teams,
            team_names: [file.teams[0].name.clone(), file.teams[1].name.clone()],
        })
    }

    /// Какая модель и оружие у солдата — по его номеру, с учётом весов.
    pub fn pick(&self, team: u8, id: u32) -> Option<(usize, usize)> {
        let list = &self.teams[team as usize & 1];
        let total: u32 = list.iter().map(|e| e.2).sum();
        if total == 0 {
            return None;
        }
        let mut r = id.wrapping_mul(2_654_435_761) >> 8;
        r %= total;
        for &(m, w, weight) in list {
            if r < weight {
                return Some((m, w));
            }
            r -= weight;
        }
        None
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

    /// Убитый: падает назад (или вперёд) за полсекунды и лежит, раскинув руки.
    pub fn corpse(&self, age: f32, forward: bool) -> Pose {
        let t = (age / 0.45).clamp(0.0, 1.0);
        let t = t * t * (3.0 - 2.0 * t);
        let angle = if forward { -1.0 } else { 1.0 } * std::f32::consts::FRAC_PI_2 * t;
        let rot = Mat3::from_rotation_z(angle);
        let lift = 3.0 * t;
        self.rest.iter().map(|&v| rot * v + Vec3::Y * lift).collect()
    }

    /// Части модели в пространстве модели (метры, +X вперёд): (меш, матрица).
    pub fn parts(&self, pose: &Pose, out: &mut impl FnMut(usize, Mat4)) {
        for part in &self.parts {
            let (a, b) = (pose[part.stick.0], pose[part.stick.1]);
            let frame = stick_frame(b - a);
            let m = Mat4::from_translation(a * VOXEL) * Mat4::from_mat3(frame * part.rest_frame.transpose());
            out(part.mesh, m);
        }
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

/// Система координат палки: ось — сама палка, второе направление — «вверх»,
/// а для почти вертикальных палок — «вперёд». Переход плавный, чтобы части не дёргались.
fn stick_frame(d: Vec3) -> Mat3 {
    let d = d.normalize_or(Vec3::Y);
    let vertical = d.dot(Vec3::Y).abs();
    let t = ((vertical - 0.8) / 0.15).clamp(0.0, 1.0);
    let hint = (Vec3::Y * (1.0 - t) + Vec3::X * t).normalize();
    let side = d.cross(hint).normalize_or(Vec3::Z);
    let up = side.cross(d);
    Mat3::from_cols(d, up, side)
}

// --- Сборка мешей и атласа ---------------------------------------------------------

/// Кусок атласа: размер и цвета.
struct Block {
    size: IVec3,
    colors: Vec<[u8; 4]>,
}

#[derive(Default)]
struct Builder {
    meshes: Vec<CharMesh>,
    blocks: Vec<(Block, IVec3)>,
    cursor: IVec3,
    row_depth: i32,
    atlas_extent: IVec3,
}

/// Атлас не шире этого по X (ограничение 3D-текстур в wgpu по умолчанию — 2048).
const ATLAS_WIDTH: i32 = 2048;

impl Builder {
    fn character(&mut self, name: &str, m: &RwrModel) -> CharModel {
        let rest: Vec<Vec3> = m.particles.iter().map(|p| p.pos).collect();
        // Все воксели модели: грань части, закрытая вокселем соседней части, не нужна.
        let solid: HashSet<IVec3> = m.voxels.iter().map(|v| v.0).collect();
        let mut parts = Vec::new();
        for (s, &(a, b)) in m.sticks.iter().enumerate() {
            let voxels: Vec<_> = m.bindings[s].iter().map(|&i| m.voxels[i]).collect();
            if voxels.is_empty() {
                continue;
            }
            let origin = rest[a];
            let mesh = self.mesh(&voxels, origin, Some(&solid));
            parts.push(Part { stick: (a, b), mesh, rest_origin: origin, rest_frame: stick_frame(rest[b] - rest[a]) });
        }
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
        CharModel { name: name.to_owned(), rest, parts, joints }
    }

    /// Предмет без скелета (оружие): одна часть с началом координат модели.
    fn prop(&mut self, m: &RwrModel) -> usize {
        self.mesh(&m.voxels, Vec3::ZERO, None)
    }

    /// Меш из вокселей: грани склеиваются по форме, цвета уходят в атлас.
    fn mesh(&mut self, voxels: &[(IVec3, [u8; 4])], origin: Vec3, whole: Option<&HashSet<IVec3>>) -> usize {
        let min = voxels.iter().fold(IVec3::MAX, |a, v| a.min(v.0));
        let max = voxels.iter().fold(IVec3::MIN, |a, v| a.max(v.0));
        let size = max - min + IVec3::ONE;
        let mut colors = vec![[0u8; 4]; (size.x * size.y * size.z) as usize];
        let idx = |p: IVec3| ((p.z * size.y + p.y) * size.x + p.x) as usize;
        for &(c, rgba) in voxels {
            colors[idx(c - min)] = [rgba[0], rgba[1], rgba[2], 255];
        }
        let place = self.place(size);
        let filled = |p: IVec3| {
            p.cmpge(IVec3::ZERO).all() && p.cmplt(size).all() && colors[idx(p)][3] != 0
        };
        let mut mesh = CharMesh::default();
        let solid = |p: IVec3| filled(p) || whole.is_some_and(|w| w.contains(&(p + min)));
        for (corners, normal) in greedy_shape(size, filled, solid) {
            let base = mesh.vertices.len() as u32;
            for c in corners {
                let world = (c + min).as_vec3();
                let a = c + place;
                mesh.vertices.push(CharVertex {
                    pos: ((world - origin) * VOXEL).to_array(),
                    normal: [normal[0], normal[1], normal[2], 0],
                    atlas: [a.x as u16, a.y as u16, a.z as u16, 0],
                });
            }
            mesh.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }
        self.blocks.push((Block { size, colors }, place));
        self.meshes.push(mesh);
        self.meshes.len() - 1
    }

    /// Место для блока в атласе: ряды вдоль X, с зазором в 1 воксель.
    fn place(&mut self, size: IVec3) -> IVec3 {
        if self.cursor.x + size.x > ATLAS_WIDTH {
            self.cursor = IVec3::new(0, 0, self.cursor.z + self.row_depth + 1);
            self.row_depth = 0;
        }
        let at = self.cursor;
        self.cursor.x += size.x + 1;
        self.row_depth = self.row_depth.max(size.z);
        self.atlas_extent = self.atlas_extent.max(at + size);
        at
    }

    fn finish_atlas(&self) -> ([u32; 3], Vec<u8>) {
        let e = self.atlas_extent.max(IVec3::ONE);
        let mut data = vec![0u8; (e.x * e.y * e.z * 4) as usize];
        for (block, at) in &self.blocks {
            let s = block.size;
            for z in 0..s.z {
                for y in 0..s.y {
                    for x in 0..s.x {
                        let src = block.colors[((z * s.y + y) * s.x + x) as usize];
                        let p = *at + IVec3::new(x, y, z);
                        let dst = (((p.z * e.y + p.y) * e.x + p.x) * 4) as usize;
                        data[dst..dst + 4].copy_from_slice(&src);
                    }
                }
            }
        }
        ([e.x as u32, e.y as u32, e.z as u32], data)
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
        assert_eq!(m.parts.len(), 17);
        let quads: usize = m.parts.iter().map(|p| set.meshes[p.mesh].indices.len() / 6).sum();
        // Склейка по форме: сотни граней, а не тысячи.
        assert!(quads < 3500, "граней у солдата: {quads}");
        println!("граней у солдата: {quads}");
        assert!(set.atlas_size.iter().all(|&s| s <= 2048));
        assert!(set.pick(0, 5).is_some() && set.pick(1, 5).is_some());
    }

    #[test]
    fn rest_pose_keeps_parts_in_place() {
        let set = CharacterSet::load(&data()).unwrap();
        let m = &set.models[0];
        let mut moved = 0.0f32;
        m.parts(&m.rest, &mut |_, mat| {
            // В позе покоя часть переходит в своё же место: матрица = перенос в точку крепления.
            let rot = Mat3::from_mat4(mat);
            moved = moved.max((rot - Mat3::IDENTITY).abs().to_cols_array().iter().copied().fold(0.0, f32::max));
        });
        assert!(moved < 1e-4, "{moved}");
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
