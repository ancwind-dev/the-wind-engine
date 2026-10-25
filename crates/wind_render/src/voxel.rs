//! Воксельные модели: формат MagicaVoxel (`.vox`) и склейка в меш.
//!
//! У нас ось Y смотрит вверх, а в MagicaVoxel вверх смотрит Z — при чтении
//! и записи оси меняются местами.

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use std::path::Path;

/// Цвет палитры, который при отрисовке заменяется цветом команды.
pub const TEAM_COLOR: [u8; 4] = [255, 0, 255, 255];

#[derive(Clone, Debug)]
pub struct VoxelModel {
    /// Размер по X (вперёд), Y (вверх), Z (вбок).
    pub size: [u32; 3],
    /// Номер цвета в палитре; 0 — пусто.
    pub voxels: Vec<u8>,
    /// `palette[i]` — цвет номера `i` (RGBA). `palette[0]` не используется.
    pub palette: Vec<[u8; 4]>,
}

impl VoxelModel {
    pub fn new(size: [u32; 3]) -> Self {
        VoxelModel {
            size,
            voxels: vec![0; (size[0] * size[1] * size[2]) as usize],
            palette: vec![[0, 0, 0, 0]; 256],
        }
    }

    fn index(&self, x: i32, y: i32, z: i32) -> Option<usize> {
        let [sx, sy, sz] = self.size.map(|s| s as i32);
        if x < 0 || y < 0 || z < 0 || x >= sx || y >= sy || z >= sz {
            return None;
        }
        Some(((z * sy + y) * sx + x) as usize)
    }

    pub fn get(&self, x: i32, y: i32, z: i32) -> u8 {
        self.index(x, y, z).map_or(0, |i| self.voxels[i])
    }

    pub fn set(&mut self, x: i32, y: i32, z: i32, color: u8) {
        if let Some(i) = self.index(x, y, z) {
            self.voxels[i] = color;
        }
    }

    /// Заливает параллелепипед [from, to) цветом.
    pub fn fill(&mut self, from: [i32; 3], to: [i32; 3], color: u8) {
        for z in from[2]..to[2] {
            for y in from[1]..to[1] {
                for x in from[0]..to[0] {
                    self.set(x, y, z, color);
                }
            }
        }
    }

    pub fn count(&self) -> usize {
        self.voxels.iter().filter(|&&v| v != 0).count()
    }

    // --- .vox --------------------------------------------------------------

    pub fn load(path: &Path) -> Result<VoxelModel, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        Self::from_vox(&bytes).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(path, self.to_vox()).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Первая модель из файла `.vox`.
    pub fn from_vox(bytes: &[u8]) -> Result<VoxelModel, String> {
        if bytes.len() < 8 || &bytes[..4] != b"VOX " {
            return Err("это не файл MagicaVoxel".into());
        }
        let mut size = None;
        let mut points: Vec<[u8; 4]> = Vec::new();
        let mut rgba: Option<Vec<[u8; 4]>> = None;
        // Пропускаем заголовок и MAIN, дальше идут его дети подряд.
        let mut pos = 8 + 12;
        while pos + 12 <= bytes.len() {
            let id = &bytes[pos..pos + 4];
            let content = read_i32(bytes, pos + 4) as usize;
            let children = read_i32(bytes, pos + 8) as usize;
            let body = pos + 12;
            if body + content > bytes.len() {
                return Err("файл обрезан".into());
            }
            match id {
                b"SIZE" if size.is_none() => {
                    size = Some([read_i32(bytes, body), read_i32(bytes, body + 4), read_i32(bytes, body + 8)]);
                }
                b"XYZI" if points.is_empty() => {
                    let n = read_i32(bytes, body) as usize;
                    if body + 4 + n * 4 > bytes.len() {
                        return Err("файл обрезан".into());
                    }
                    points = (0..n)
                        .map(|k| {
                            let p = body + 4 + k * 4;
                            [bytes[p], bytes[p + 1], bytes[p + 2], bytes[p + 3]]
                        })
                        .collect();
                }
                b"RGBA" => {
                    rgba = Some(
                        (0..256)
                            .map(|k| {
                                let p = body + k * 4;
                                [bytes[p], bytes[p + 1], bytes[p + 2], bytes[p + 3]]
                            })
                            .collect(),
                    );
                }
                _ => {}
            }
            pos = body + content + if id == b"MAIN" { 0 } else { children };
        }
        let [sx, sy, sz] = size.ok_or("нет SIZE")?;
        // MagicaVoxel: Z вверх → у нас Y вверх.
        let mut model = VoxelModel::new([sx as u32, sz as u32, sy as u32]);
        for [x, y, z, c] in points {
            model.set(x as i32, z as i32, y as i32, c);
        }
        match rgba {
            // В файле цвет номера i лежит на месте i - 1.
            Some(colors) => model.palette[1..256].copy_from_slice(&colors[..255]),
            None => {
                for i in 1..256 {
                    model.palette[i] = [200, 200, 200, 255];
                }
            }
        }
        Ok(model)
    }

    pub fn to_vox(&self) -> Vec<u8> {
        let mut xyzi = Vec::new();
        let mut n = 0i32;
        for z in 0..self.size[2] as i32 {
            for y in 0..self.size[1] as i32 {
                for x in 0..self.size[0] as i32 {
                    let c = self.get(x, y, z);
                    if c != 0 {
                        // У нас Y вверх → в файле Z вверх.
                        xyzi.extend_from_slice(&[x as u8, z as u8, y as u8, c]);
                        n += 1;
                    }
                }
            }
        }
        let mut children = Vec::new();
        chunk(&mut children, b"SIZE", &[
            &(self.size[0] as i32).to_le_bytes()[..],
            &(self.size[2] as i32).to_le_bytes()[..],
            &(self.size[1] as i32).to_le_bytes()[..],
        ]
        .concat());
        chunk(&mut children, b"XYZI", &[&n.to_le_bytes()[..], &xyzi[..]].concat());
        let mut rgba = Vec::with_capacity(1024);
        for i in 1..=256 {
            rgba.extend_from_slice(&self.palette.get(i).copied().unwrap_or([0, 0, 0, 255]));
        }
        chunk(&mut children, b"RGBA", &rgba);

        let mut out = Vec::new();
        out.extend_from_slice(b"VOX ");
        out.extend_from_slice(&150i32.to_le_bytes());
        out.extend_from_slice(b"MAIN");
        out.extend_from_slice(&0i32.to_le_bytes());
        out.extend_from_slice(&(children.len() as i32).to_le_bytes());
        out.extend_from_slice(&children);
        out
    }
}

fn read_i32(bytes: &[u8], at: usize) -> i32 {
    i32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

fn chunk(out: &mut Vec<u8>, id: &[u8; 4], content: &[u8]) {
    out.extend_from_slice(id);
    out.extend_from_slice(&(content.len() as i32).to_le_bytes());
    out.extend_from_slice(&0i32.to_le_bytes());
    out.extend_from_slice(content);
}

// --- Меш ------------------------------------------------------------------

/// Вершина: позиция, нормаль, цвет. 20 байт.
/// Альфа цвета 0 — «покрасить в цвет команды».
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [i8; 4],
    pub color: [u8; 4],
}

#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl Mesh {
    /// Четырёхугольник; вершины — против часовой стрелки, если смотреть со стороны нормали.
    pub fn quad(&mut self, corners: [Vec3; 4], normal: [i8; 3], colors: [[u8; 4]; 4]) {
        let base = self.vertices.len() as u32;
        for (p, c) in corners.into_iter().zip(colors) {
            self.vertices.push(Vertex { pos: p.to_array(), normal: [normal[0], normal[1], normal[2], 0], color: c });
        }
        self.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
    }

    pub fn quads(&self) -> usize {
        self.indices.len() / 6
    }
}

/// Жадная склейка: соседние грани одного цвета и направления сливаются в один
/// прямоугольник. Куб 8×8×8 даёт 6 четырёхугольников, а не 384.
///
/// `pivot` — точка модели (в вокселях), которая станет началом координат меша;
/// `scale` — размер вокселя в метрах.
pub fn greedy_mesh(model: &VoxelModel, pivot: Vec3, scale: f32) -> Mesh {
    let mut mesh = Mesh::default();
    let size = model.size.map(|s| s as i32);
    let color_of = |c: u8| {
        let p = model.palette[c as usize];
        if p == TEAM_COLOR { [255, 255, 255, 0] } else { [p[0], p[1], p[2], 255] }
    };

    for d in 0..3 {
        let u = (d + 1) % 3;
        let v = (d + 2) % 3;
        let mut x = [0i32; 3];
        let mut q = [0i32; 3];
        q[d] = 1;
        // Маска граней в срезе: знак — направление нормали, модуль — номер цвета.
        let mut mask = vec![0i32; (size[u] * size[v]) as usize];
        x[d] = -1;
        while x[d] < size[d] {
            let mut n = 0;
            x[v] = 0;
            while x[v] < size[v] {
                x[u] = 0;
                while x[u] < size[u] {
                    let a = model.get(x[0], x[1], x[2]) as i32;
                    let b = model.get(x[0] + q[0], x[1] + q[1], x[2] + q[2]) as i32;
                    mask[n] = match (a != 0, b != 0) {
                        (true, false) => a,
                        (false, true) => -b,
                        _ => 0,
                    };
                    n += 1;
                    x[u] += 1;
                }
                x[v] += 1;
            }
            x[d] += 1;

            // Собираем прямоугольники из одинаковых клеток маски.
            let mut n = 0;
            for j in 0..size[v] {
                let mut i = 0;
                while i < size[u] {
                    let c = mask[n];
                    if c == 0 {
                        i += 1;
                        n += 1;
                        continue;
                    }
                    let mut w = 1;
                    while i + w < size[u] && mask[n + w as usize] == c {
                        w += 1;
                    }
                    let mut h = 1;
                    'grow: while j + h < size[v] {
                        for k in 0..w {
                            if mask[n + (k + h * size[u]) as usize] != c {
                                break 'grow;
                            }
                        }
                        h += 1;
                    }

                    x[u] = i;
                    x[v] = j;
                    let mut du = [0i32; 3];
                    du[u] = w;
                    let mut dv = [0i32; 3];
                    dv[v] = h;
                    let p = |o: [i32; 3]| {
                        (Vec3::new((x[0] + o[0]) as f32, (x[1] + o[1]) as f32, (x[2] + o[2]) as f32) - pivot) * scale
                    };
                    let p0 = p([0, 0, 0]);
                    let p1 = p(du);
                    let p2 = p([du[0] + dv[0], du[1] + dv[1], du[2] + dv[2]]);
                    let p3 = p(dv);
                    let color = color_of(c.unsigned_abs() as u8);
                    let mut normal = [0i8; 3];
                    if c > 0 {
                        normal[d] = 127;
                        mesh.quad([p0, p1, p2, p3], normal, [color; 4]);
                    } else {
                        normal[d] = -127;
                        mesh.quad([p0, p3, p2, p1], normal, [color; 4]);
                    }

                    for l in 0..h {
                        for k in 0..w {
                            mask[n + (k + l * size[u]) as usize] = 0;
                        }
                    }
                    i += w;
                    n += w as usize;
                }
            }
        }
    }
    mesh
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_is_six_quads() {
        let mut m = VoxelModel::new([8, 8, 8]);
        m.palette[1] = [10, 20, 30, 255];
        m.fill([0, 0, 0], [8, 8, 8], 1);
        let mesh = greedy_mesh(&m, Vec3::ZERO, 1.0);
        assert_eq!(mesh.quads(), 6);
    }

    #[test]
    fn colors_split_faces() {
        let mut m = VoxelModel::new([2, 1, 1]);
        m.palette[1] = [255, 0, 0, 255];
        m.palette[2] = [0, 255, 0, 255];
        m.set(0, 0, 0, 1);
        m.set(1, 0, 0, 2);
        // Сверху, снизу, спереди, сзади — по две грани; торцы — по одной.
        assert_eq!(greedy_mesh(&m, Vec3::ZERO, 1.0).quads(), 10);
    }

    #[test]
    fn vox_round_trip() {
        let mut m = VoxelModel::new([3, 5, 2]);
        m.palette[7] = [1, 2, 3, 255];
        m.palette[9] = TEAM_COLOR;
        m.set(0, 4, 1, 7);
        m.set(2, 0, 0, 9);
        let back = VoxelModel::from_vox(&m.to_vox()).unwrap();
        assert_eq!(back.size, m.size);
        assert_eq!(back.voxels, m.voxels);
        assert_eq!(back.palette[7], [1, 2, 3, 255]);
        assert_eq!(back.palette[9], TEAM_COLOR);
    }
}
