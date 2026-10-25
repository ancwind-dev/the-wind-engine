//! Чтение моделей в формате RWR (`.xml`): воксели, скелет из точек и палок,
//! привязка вокселей к палкам. Так выгружают модели скрипты `gallery/my game/tools`.
//!
//! Оси приводятся к нашим: у RWR модель смотрит в +Z, а у нас вперёд — +X,
//! вверх в обоих случаях Y. Точка RWR (x, y, z) у нас — (z, y, −x).
//! Размер вокселя — 1/32 м.

use glam::{IVec3, Vec3};

pub const VOXEL: f32 = 1.0 / 32.0;

#[derive(Clone, Debug)]
pub struct Particle {
    pub name: String,
    /// В вокселях, уже в наших осях.
    pub pos: Vec3,
}

#[derive(Clone, Debug, Default)]
pub struct RwrModel {
    /// Клетка вокселя (в наших осях) и цвет.
    pub voxels: Vec<(IVec3, [u8; 4])>,
    pub particles: Vec<Particle>,
    /// Палки: номера точек.
    pub sticks: Vec<(usize, usize)>,
    /// Для каждой палки — номера её вокселей.
    pub bindings: Vec<Vec<usize>>,
}

impl RwrModel {
    pub fn particle(&self, name: &str) -> Option<usize> {
        self.particles.iter().position(|p| p.name == name)
    }
}

/// Атрибуты одного тега: `<voxel x="1" y="2" ... />`.
fn attrs(tag: &str) -> impl Iterator<Item = (&str, &str)> {
    let mut rest = tag;
    std::iter::from_fn(move || {
        let eq = rest.find("=\"")?;
        let key = rest[..eq].trim().rsplit(char::is_whitespace).next()?;
        let after = &rest[eq + 2..];
        let end = after.find('"')?;
        let value = &after[..end];
        rest = &after[end + 1..];
        Some((key, value))
    })
}

fn num<T: std::str::FromStr>(v: &str, what: &str) -> Result<T, String> {
    v.parse().map_err(|_| format!("{what}: не число «{v}»"))
}

/// RWR → наши оси для точки.
fn axes(x: f32, y: f32, z: f32) -> Vec3 {
    Vec3::new(z, y, -x)
}

pub fn parse(text: &str) -> Result<RwrModel, String> {
    let mut model = RwrModel::default();
    let mut ids: Vec<i64> = Vec::new();
    let mut raw_sticks: Vec<(i64, i64)> = Vec::new();
    let mut in_bindings = false;
    let mut group: Option<usize> = None;

    for piece in text.split('<').skip(1) {
        let tag = piece.split('>').next().unwrap_or("");
        let name = tag.split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("");
        match name {
            "skeletonVoxelBindings" => in_bindings = true,
            "group" => {
                for (k, v) in attrs(tag) {
                    if k == "constraintIndex" {
                        let i: usize = num(v, "constraintIndex")?;
                        if model.bindings.len() <= i {
                            model.bindings.resize(i + 1, Vec::new());
                        }
                        group = Some(i);
                    }
                }
            }
            "voxel" if in_bindings => {
                if let (Some(g), Some((_, v))) = (group, attrs(tag).find(|(k, _)| *k == "index")) {
                    model.bindings[g].push(num(v, "index")?);
                }
            }
            "voxel" => {
                let (mut p, mut c) = ([0i32; 3], [0f32; 4]);
                c[3] = 1.0;
                for (k, v) in attrs(tag) {
                    match k {
                        "x" => p[0] = num(v, "x")?,
                        "y" => p[1] = num(v, "y")?,
                        "z" => p[2] = num(v, "z")?,
                        "r" => c[0] = num(v, "r")?,
                        "g" => c[1] = num(v, "g")?,
                        "b" => c[2] = num(v, "b")?,
                        "a" => c[3] = num(v, "a")?,
                        _ => {}
                    }
                }
                // Клетка [x, x+1] по X у нас становится [−x−1, −x] по Z.
                let cell = IVec3::new(p[2], p[1], -p[0] - 1);
                let rgba = c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
                model.voxels.push((cell, rgba));
            }
            "particle" => {
                let (mut p, mut id, mut pname) = ([0f32; 3], 0i64, String::new());
                for (k, v) in attrs(tag) {
                    match k {
                        "x" => p[0] = num(v, "x")?,
                        "y" => p[1] = num(v, "y")?,
                        "z" => p[2] = num(v, "z")?,
                        "id" => id = num(v, "id")?,
                        "name" => pname = v.to_owned(),
                        _ => {}
                    }
                }
                ids.push(id);
                model.particles.push(Particle { name: pname, pos: axes(p[0], p[1], p[2]) });
            }
            "stick" => {
                let (mut a, mut b) = (0i64, 0i64);
                for (k, v) in attrs(tag) {
                    match k {
                        "a" => a = num(v, "a")?,
                        "b" => b = num(v, "b")?,
                        _ => {}
                    }
                }
                raw_sticks.push((a, b));
            }
            _ => {}
        }
    }

    let index = |id: i64| ids.iter().position(|&x| x == id).ok_or(format!("палка ссылается на неизвестную точку {id}"));
    for (a, b) in raw_sticks {
        model.sticks.push((index(a)?, index(b)?));
    }
    model.bindings.resize(model.sticks.len(), Vec::new());
    if model.voxels.is_empty() {
        return Err("в модели нет вокселей".into());
    }
    for group in &model.bindings {
        if group.iter().any(|&i| i >= model.voxels.len()) {
            return Err("привязка ссылается на несуществующий воксель".into());
        }
    }
    Ok(model)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_tiny_model() {
        let text = r#"<model><voxels>
            <voxel x="0" y="0" z="0" r="1.0" g="0.5" b="0" a="1" />
            <voxel x="-2" y="1" z="3" r="0" g="0" b="1" a="1" />
            </voxels><skeleton>
            <particle x="0.5" y="0.5" z="0.5" invMass="1" id="7" name="leftfoot" bodyAreaHint="1" />
            <particle x="0.5" y="5.5" z="0.5" invMass="1" id="9" name="leftknee" bodyAreaHint="1" />
            <stick a="7" b="9" />
            </skeleton><skeletonVoxelBindings><group constraintIndex="0">
            <voxel index="1" /><voxel index="0" /></group></skeletonVoxelBindings></model>"#;
        let m = parse(text).unwrap();
        assert_eq!(m.voxels.len(), 2);
        assert_eq!(m.voxels[0], (IVec3::new(0, 0, -1), [255, 128, 0, 255]));
        assert_eq!(m.voxels[1].0, IVec3::new(3, 1, 1));
        assert_eq!(m.particle("leftknee"), Some(1));
        assert_eq!(m.sticks, vec![(0, 1)]);
        assert_eq!(m.bindings, vec![vec![1, 0]]);
        assert_eq!(m.particles[1].pos, Vec3::new(0.5, 5.5, -0.5));
    }

    #[test]
    fn reads_real_soldier() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../data/models/soldiers/usmc/rifleman.xml");
        let m = parse(&std::fs::read_to_string(path).unwrap()).unwrap();
        assert_eq!(m.particles.len(), 15);
        assert_eq!(m.sticks.len(), 17);
        assert!(m.voxels.len() > 1000);
        let bound: usize = m.bindings.iter().map(Vec::len).sum();
        assert_eq!(bound, m.voxels.len(), "каждый воксель привязан ровно к одной палке");
        // Правое плечо у нас со стороны +Z (вперёд +X, вверх +Y).
        let right = m.particles[m.particle("rightshoulder").unwrap()].pos;
        assert!(right.z > 0.0, "{right}");
    }
}
