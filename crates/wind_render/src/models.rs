//! Модели солдата из жёстких частей и куб для эффектов.
//!
//! Встроенные части собираются в коде. Если в `data/models/soldier/` лежит
//! `.vox` с тем же именем — берётся он: так модели можно рисовать в MagicaVoxel.

use crate::voxel::{Mesh, TEAM_COLOR, VoxelModel, greedy_mesh};
use glam::Vec3;
use std::path::Path;

/// Размер вокселя солдата, м. Солдат ~21 воксель ростом → ~1,8 м.
pub const SOLDIER_VOXEL: f32 = 0.085;

/// Меши, которые рисуются экземплярами. Порядок = номер меша.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshId {
    Head,
    Torso,
    Arm,
    Leg,
    Rifle,
    Cube,
    Boat,
}

pub const MESH_COUNT: usize = 7;

/// Размер вокселя лодки, м.
pub const BOAT_VOXEL: f32 = 0.12;
/// Дно лодки ниже воды на столько, м; пол (по нему стоят сидящие) — над водой, на два
/// вокселя выше дна: иначе вода рисуется поверх ног.
pub const BOAT_SINK: f32 = 0.15;
pub const BOAT_DECK: f32 = 2.0 * BOAT_VOXEL - BOAT_SINK;

/// Часть солдата: имя файла, точка крепления в модели (в вокселях).
pub struct Part {
    pub id: MeshId,
    pub name: &'static str,
    pub pivot: Vec3,
    pub build: fn() -> VoxelModel,
}

pub const PARTS: [Part; 5] = [
    Part { id: MeshId::Head, name: "head", pivot: Vec3::new(3.0, 0.0, 3.0), build: head },
    Part { id: MeshId::Torso, name: "torso", pivot: Vec3::new(2.0, 0.0, 3.5), build: torso },
    Part { id: MeshId::Arm, name: "arm", pivot: Vec3::new(1.0, 6.0, 1.0), build: arm },
    Part { id: MeshId::Leg, name: "leg", pivot: Vec3::new(1.5, 8.0, 1.5), build: leg },
    Part { id: MeshId::Rifle, name: "rifle", pivot: Vec3::new(3.0, 1.0, 0.5), build: rifle },
];

// Номера цветов во встроенной палитре.
const TEAM: u8 = 1;
const SKIN: u8 = 2;
const HELMET: u8 = 3;
const BOOTS: u8 = 4;
const BELT: u8 = 5;
const METAL: u8 = 6;
const WOOD: u8 = 7;
const EYES: u8 = 8;

fn palette(m: &mut VoxelModel) {
    m.palette[TEAM as usize] = TEAM_COLOR;
    m.palette[SKIN as usize] = [222, 176, 140, 255];
    m.palette[HELMET as usize] = [74, 84, 58, 255];
    m.palette[BOOTS as usize] = [52, 42, 34, 255];
    m.palette[BELT as usize] = [40, 40, 36, 255];
    m.palette[METAL as usize] = [52, 54, 58, 255];
    m.palette[WOOD as usize] = [112, 76, 46, 255];
    m.palette[EYES as usize] = [30, 30, 30, 255];
}

fn model(size: [u32; 3]) -> VoxelModel {
    let mut m = VoxelModel::new(size);
    palette(&mut m);
    m
}

fn head() -> VoxelModel {
    let mut m = model([6, 7, 6]);
    m.fill([1, 0, 1], [5, 4, 5], SKIN);
    m.fill([0, 3, 0], [6, 6, 6], HELMET);
    m.fill([1, 6, 1], [5, 7, 5], HELMET);
    m.set(4, 2, 2, EYES);
    m.set(4, 2, 3, EYES);
    m
}

fn torso() -> VoxelModel {
    let mut m = model([4, 7, 7]);
    m.fill([0, 0, 0], [4, 7, 7], TEAM);
    m.fill([0, 0, 0], [4, 1, 7], BELT);
    m
}

fn arm() -> VoxelModel {
    let mut m = model([2, 6, 2]);
    m.fill([0, 1, 0], [2, 6, 2], TEAM);
    m.fill([0, 0, 0], [2, 1, 2], SKIN);
    m
}

fn leg() -> VoxelModel {
    let mut m = model([4, 8, 3]);
    m.fill([0, 2, 0], [3, 8, 3], TEAM);
    m.fill([0, 0, 0], [3, 2, 3], BOOTS);
    m.fill([3, 0, 0], [4, 1, 3], BOOTS); // носок
    m
}

fn rifle() -> VoxelModel {
    let mut m = model([11, 2, 1]);
    m.fill([0, 0, 0], [3, 2, 1], WOOD);
    m.fill([3, 1, 0], [11, 2, 1], METAL);
    m.fill([4, 0, 0], [6, 1, 1], METAL); // магазин
    m
}

/// Меши всех частей, по порядку [`MeshId`]. Файлы из `models_dir` важнее встроенных.
pub fn build_meshes(models_dir: Option<&Path>) -> (Vec<Mesh>, Vec<String>) {
    let mut meshes = Vec::with_capacity(MESH_COUNT);
    let mut notes = Vec::new();
    for part in &PARTS {
        let file = models_dir.map(|d| d.join(format!("{}.vox", part.name)));
        let vox = match file.as_deref().filter(|f| f.exists()).map(VoxelModel::load) {
            Some(Ok(m)) => {
                notes.push(format!("модель {} из {}", part.name, file.unwrap().display()));
                m
            }
            Some(Err(e)) => {
                notes.push(format!("не удалось прочитать {}: {e}, беру встроенную", part.name));
                (part.build)()
            }
            None => (part.build)(),
        };
        meshes.push(greedy_mesh(&vox, part.pivot, SOLDIER_VOXEL));
    }
    meshes.push(cube());
    meshes.push(greedy_mesh(&boat(), Vec3::new(18.0, 0.0, 7.5), BOAT_VOXEL));
    (meshes, notes)
}

/// Сохраняет встроенные части в `.vox`, чтобы их можно было открыть в MagicaVoxel.
pub fn export(models_dir: &Path) -> Result<Vec<String>, String> {
    PARTS
        .iter()
        .map(|part| {
            let path = models_dir.join(format!("{}.vox", part.name));
            (part.build)().save(&path)?;
            Ok(path.display().to_string())
        })
        .collect()
}

/// Штурмовая лодка: 36 × 15 вокселей (4,3 × 1,8 м), нос — вперёд (+x), острый; борта —
/// толстые, светлее по верху; внутри — пол; на корме — мотор.
fn boat() -> VoxelModel {
    const HULL: u8 = 1;
    const RIM: u8 = 2;
    const FLOOR: u8 = 3;
    const MOTOR: u8 = 4;
    const SEAT: u8 = 5;
    let mut m = VoxelModel::new([36, 8, 15]);
    m.palette[HULL as usize] = [70, 78, 58, 255];
    m.palette[RIM as usize] = [98, 106, 82, 255];
    m.palette[FLOOR as usize] = [52, 56, 46, 255];
    m.palette[MOTOR as usize] = [38, 38, 40, 255];
    m.palette[SEAT as usize] = [84, 70, 52, 255];
    for x in 3..36i32 {
        // Ширина: на корме и в середине — полная, к носу сужается.
        let t = ((x as f32 - 24.0) / 12.0).clamp(0.0, 1.0);
        let half = 7.5 * (1.0 - t * t * 0.85);
        let (z0, z1) = ((7.5 - half).round() as i32, (7.5 + half).round() as i32);
        // Нос чуть приподнят.
        let lift = if x > 30 { 1 } else { 0 };
        for z in z0..z1 {
            m.set(x, lift, z, HULL);
            let side = z < z0 + 2 || z + 2 >= z1 || x < 5;
            for y in 1 + lift..4 + lift {
                if side {
                    m.set(x, y, z, if y == 3 + lift { RIM } else { HULL });
                } else if y == 1 + lift {
                    m.set(x, y, z, FLOOR);
                }
            }
        }
    }
    // Банка поперёк — сиденье в середине.
    m.fill([17, 2, 2], [19, 3, 13], SEAT);
    // Мотор на корме: колонка в воду и кожух над бортом.
    m.fill([0, 0, 6], [3, 3, 9], MOTOR);
    m.fill([0, 3, 5], [4, 7, 10], MOTOR);
    m
}

/// Единичный белый куб с центром в начале координат — для пуль, гранат, взрывов.
fn cube() -> Mesh {
    let mut m = VoxelModel::new([1, 1, 1]);
    m.palette[1] = [255, 255, 255, 255];
    m.set(0, 0, 0, 1);
    greedy_mesh(&m, Vec3::splat(0.5), 1.0)
}
