//! Модели из Blender: чтение `.glb` (glTF 2.0 одним файлом).
//!
//! Берём только то, что нужно low-poly модели с плоскими цветами: треугольники, нормали,
//! цвет — из материала (`baseColorFactor`) и цвета вершин (`COLOR_0`), если есть.
//! Текстуры, анимации и скелеты пропускаем. Узлы сцены со своими перемещениями,
//! поворотами и масштабом сводим в один меш.
//!
//! glTF, как и движок, — «Y вверх», в метрах; Blender при выгрузке поворачивает сам.

use crate::voxel::{Mesh, Vertex};
use glam::{Mat3, Mat4, Quat, Vec3, Vec4};

/// Меш из файла `.glb`.
pub fn load(bytes: &[u8]) -> Result<Mesh, String> {
    let (doc, bin) = split_glb(bytes)?;
    let json = Json::parse(doc)?;
    let mut mesh = Mesh::default();
    visit_meshes(&json, &mut |m, transform| add_mesh(&json, bin, m, transform, &mut mesh))?;
    if mesh.indices.is_empty() {
        return Err("в файле нет треугольников".into());
    }
    Ok(mesh)
}

/// Меш как есть — для травы: позиции (с учётом узлов), все цвета вершин (`COLOR_0`,
/// `COLOR_1`, … — линейные, как в файле) и треугольники.
#[derive(Debug, Default)]
pub struct Raw {
    pub positions: Vec<Vec3>,
    pub colors: Vec<Vec<Vec4>>,
    pub indices: Vec<u32>,
}

pub fn load_raw(bytes: &[u8]) -> Result<Raw, String> {
    let (doc, bin) = split_glb(bytes)?;
    let json = Json::parse(doc)?;
    let mut raw = Raw::default();
    visit_meshes(&json, &mut |index, transform| {
        let m = json.get("meshes").and_then(|m| m.at(index)).ok_or(format!("нет меша {index}"))?;
        for p in m.get("primitives").map(Json::items).unwrap_or(&[]) {
            if p.get("mode").and_then(Json::as_usize).unwrap_or(4) != 4 {
                continue;
            }
            let attrs = p.get("attributes").ok_or("у примитива нет attributes")?;
            let positions = read(&json, bin, attrs.get("POSITION").and_then(Json::as_usize).ok_or("нет POSITION")?)?;
            let first = raw.positions.len() as u32;
            raw.positions.extend(positions.iter().map(|v| transform.transform_point3(v.truncate())));
            let mut k = 0;
            while let Some(a) = attrs.get(&format!("COLOR_{k}")).and_then(Json::as_usize) {
                if raw.colors.len() <= k {
                    // Цвет, которого не было у прежних примитивов, — белый у них.
                    raw.colors.push(vec![Vec4::ONE; first as usize]);
                }
                raw.colors[k].extend(read(&json, bin, a)?);
                k += 1;
            }
            for c in &mut raw.colors {
                c.resize(raw.positions.len(), Vec4::ONE);
            }
            let indices: Vec<u32> = match p.get("indices").and_then(Json::as_usize) {
                Some(a) => read(&json, bin, a)?.iter().map(|v| v.x as u32).collect(),
                None => (0..positions.len() as u32).collect(),
            };
            if indices.iter().any(|&i| i as usize >= positions.len()) {
                return Err("индекс вершины за пределами".into());
            }
            raw.indices.extend(indices.iter().map(|i| i + first));
        }
        Ok(())
    })?;
    if raw.indices.is_empty() {
        return Err("в файле нет треугольников".into());
    }
    Ok(raw)
}

/// Обойти узлы сцены: для каждого меша — его номер и положение в мире.
fn visit_meshes(json: &Json, f: &mut dyn FnMut(usize, Mat4) -> Result<(), String>) -> Result<(), String> {
    let scene = json.get("scene").and_then(Json::as_usize).unwrap_or(0);
    let roots: Vec<usize> = match json.get("scenes").and_then(|s| s.at(scene)) {
        Some(s) => s.get("nodes").map(|n| n.items().iter().filter_map(Json::as_usize).collect()).unwrap_or_default(),
        // Сцены нет — все узлы без родителя считаем корнями.
        None => (0..json.get("nodes").map_or(0, |n| n.items().len())).collect(),
    };
    for root in roots {
        add_node(json, root, Mat4::IDENTITY, f, 0)?;
    }
    Ok(())
}

/// Куски `.glb`: JSON и двоичный буфер.
fn split_glb(bytes: &[u8]) -> Result<(&str, &[u8]), String> {
    let u32_at = |i: usize| bytes.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    if u32_at(0) != Some(0x4654_6C67) {
        return Err("не glb: нет подписи glTF (нужна выгрузка «glTF Binary (.glb)»)".into());
    }
    if u32_at(4) != Some(2) {
        return Err("нужен glTF 2.0".into());
    }
    let (mut json, mut bin) = (None, &[][..]);
    let mut at = 12;
    while let (Some(len), Some(kind)) = (u32_at(at), u32_at(at + 4)) {
        let data = bytes.get(at + 8..at + 8 + len as usize).ok_or("файл обрезан")?;
        match kind {
            0x4E4F_534A => json = Some(std::str::from_utf8(data).map_err(|e| e.to_string())?),
            0x004E_4942 => bin = data,
            _ => {}
        }
        at += 8 + len as usize;
    }
    Ok((json.ok_or("нет JSON в файле")?, bin))
}

fn add_node(json: &Json, index: usize, parent: Mat4, f: &mut dyn FnMut(usize, Mat4) -> Result<(), String>, depth: u32) -> Result<(), String> {
    if depth > 64 {
        return Err("узлы зациклены".into());
    }
    let node = json.get("nodes").and_then(|n| n.at(index)).ok_or(format!("нет узла {index}"))?;
    let local = if let Some(m) = node.get("matrix") {
        let v: Vec<f32> = m.items().iter().filter_map(Json::as_f32).collect();
        if v.len() != 16 {
            return Err("matrix узла — не 16 чисел".into());
        }
        Mat4::from_cols_slice(&v)
    } else {
        let vec = |key: &str, n: usize| node.get(key).map(|a| a.items().iter().filter_map(Json::as_f32).collect::<Vec<_>>()).filter(|v| v.len() == n);
        let t = vec("translation", 3).map_or(Vec3::ZERO, |v| Vec3::new(v[0], v[1], v[2]));
        let r = vec("rotation", 4).map_or(Quat::IDENTITY, |v| Quat::from_xyzw(v[0], v[1], v[2], v[3]).normalize());
        let s = vec("scale", 3).map_or(Vec3::ONE, |v| Vec3::new(v[0], v[1], v[2]));
        Mat4::from_scale_rotation_translation(s, r, t)
    };
    let world = parent * local;
    if let Some(m) = node.get("mesh").and_then(Json::as_usize) {
        f(m, world)?;
    }
    if let Some(children) = node.get("children") {
        for c in children.items().iter().filter_map(Json::as_usize) {
            add_node(json, c, world, f, depth + 1)?;
        }
    }
    Ok(())
}

fn add_mesh(json: &Json, bin: &[u8], index: usize, transform: Mat4, mesh: &mut Mesh) -> Result<(), String> {
    let m = json.get("meshes").and_then(|m| m.at(index)).ok_or(format!("нет меша {index}"))?;
    let normal_matrix = Mat3::from_mat4(transform).inverse().transpose();
    // Зеркальный узел выворачивает треугольники — возвращаем им порядок обхода.
    let mirrored = transform.determinant() < 0.0;
    for p in m.get("primitives").map(Json::items).unwrap_or(&[]) {
        // Только треугольники (mode 4 — по умолчанию).
        if p.get("mode").and_then(Json::as_usize).unwrap_or(4) != 4 {
            continue;
        }
        let attrs = p.get("attributes").ok_or("у примитива нет attributes")?;
        let positions = read(json, bin, attrs.get("POSITION").and_then(Json::as_usize).ok_or("нет POSITION")?)?;
        let normals = match attrs.get("NORMAL").and_then(Json::as_usize) {
            Some(a) => Some(read(json, bin, a)?),
            None => None,
        };
        let colors = match attrs.get("COLOR_0").and_then(Json::as_usize) {
            Some(a) => Some(read(json, bin, a)?),
            None => None,
        };
        let base = p
            .get("material")
            .and_then(Json::as_usize)
            .and_then(|i| json.get("materials")?.at(i))
            .and_then(|mat| mat.get("pbrMetallicRoughness")?.get("baseColorFactor"))
            .map(|f| f.items().iter().filter_map(Json::as_f32).collect::<Vec<_>>())
            .filter(|f| f.len() == 4)
            .map_or(Vec4::ONE, |f| Vec4::new(f[0], f[1], f[2], f[3]));
        let indices: Vec<u32> = match p.get("indices").and_then(Json::as_usize) {
            Some(a) => read(json, bin, a)?.iter().map(|v| v.x as u32).collect(),
            None => (0..positions.len() as u32).collect(),
        };
        if indices.iter().any(|&i| i as usize >= positions.len()) {
            return Err("индекс вершины за пределами".into());
        }

        // Без нормалей — плоские, по треугольнику: вершины не делим, каждая — своя.
        let flat = normals.is_none();
        let first = mesh.vertices.len() as u32;
        let vertex = |i: usize, n: Vec3| {
            let pos = transform.transform_point3(positions[i].truncate());
            let n = (normal_matrix * n).normalize_or_zero();
            // Цвета glTF — линейные; движок хранит sRGB, как у вокселей.
            let c = colors.as_ref().map_or(Vec4::ONE, |c| if c[i].w == 0.0 && c[i].truncate() != Vec3::ZERO { c[i].truncate().extend(1.0) } else { c[i] }) * base;
            Vertex {
                pos: pos.to_array(),
                normal: [(n.x * 127.0).round() as i8, (n.y * 127.0).round() as i8, (n.z * 127.0).round() as i8, 0],
                color: [srgb(c.x), srgb(c.y), srgb(c.z), 255],
            }
        };
        if flat {
            for &[a, b, c] in indices.as_chunks::<3>().0 {
                let [a, b, c] = [a as usize, b as usize, c as usize];
                let (pa, pb, pc) = (positions[a].truncate(), positions[b].truncate(), positions[c].truncate());
                let n = (pb - pa).cross(pc - pa).normalize_or_zero();
                let base = mesh.vertices.len() as u32;
                for i in [a, b, c] {
                    mesh.vertices.push(vertex(i, n));
                }
                mesh.indices.extend(if mirrored { [base, base + 2, base + 1] } else { [base, base + 1, base + 2] });
            }
        } else {
            for (i, n) in normals.as_ref().unwrap().iter().enumerate().take(positions.len()) {
                mesh.vertices.push(vertex(i, n.truncate()));
            }
            for &[a, b, c] in indices.as_chunks::<3>().0 {
                let [a, b, c] = [a + first, b + first, c + first];
                mesh.indices.extend(if mirrored { [a, c, b] } else { [a, b, c] });
            }
        }
    }
    Ok(())
}

/// Значения аксессора как векторы (недостающие компоненты — 0, у цвета без альфы — 1).
fn read(json: &Json, bin: &[u8], index: usize) -> Result<Vec<Vec4>, String> {
    let acc = json.get("accessors").and_then(|a| a.at(index)).ok_or(format!("нет аксессора {index}"))?;
    let count = acc.get("count").and_then(Json::as_usize).ok_or("у аксессора нет count")?;
    let comps = match acc.get("type").and_then(Json::as_str) {
        Some("SCALAR") => 1,
        Some("VEC2") => 2,
        Some("VEC3") => 3,
        Some("VEC4") => 4,
        other => return Err(format!("тип аксессора {other:?} не поддерживается")),
    };
    let kind = acc.get("componentType").and_then(Json::as_usize).ok_or("нет componentType")?;
    let normalized = acc.get("normalized").and_then(Json::as_bool).unwrap_or(false);
    let size = match kind {
        5120 | 5121 => 1,
        5122 | 5123 => 2,
        5125 | 5126 => 4,
        _ => return Err(format!("componentType {kind} не поддерживается")),
    };
    let Some(view_index) = acc.get("bufferView").and_then(Json::as_usize) else {
        // Аксессор без буфера — нули (так бывает у разреженных; их не поддерживаем).
        return Ok(vec![Vec4::ZERO; count]);
    };
    let view = json.get("bufferViews").and_then(|v| v.at(view_index)).ok_or(format!("нет bufferView {view_index}"))?;
    if view.get("buffer").and_then(Json::as_usize).unwrap_or(0) != 0 {
        return Err("внешние буферы не поддерживаются — нужен .glb".into());
    }
    let offset = view.get("byteOffset").and_then(Json::as_usize).unwrap_or(0) + acc.get("byteOffset").and_then(Json::as_usize).unwrap_or(0);
    let stride = view.get("byteStride").and_then(Json::as_usize).unwrap_or(comps * size);
    let mut out = Vec::with_capacity(count);
    for i in 0..count {
        let mut v = [0.0f32, 0.0, 0.0, if comps == 3 { 1.0 } else { 0.0 }];
        for (c, slot) in v.iter_mut().enumerate().take(comps) {
            let at = offset + i * stride + c * size;
            let b = bin.get(at..at + size).ok_or("аксессор выходит за буфер")?;
            *slot = match kind {
                5126 => f32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                5125 => u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as f32,
                5123 => {
                    let x = u16::from_le_bytes([b[0], b[1]]) as f32;
                    if normalized { x / 65535.0 } else { x }
                }
                5122 => {
                    let x = i16::from_le_bytes([b[0], b[1]]) as f32;
                    if normalized { (x / 32767.0).max(-1.0) } else { x }
                }
                5121 => {
                    let x = b[0] as f32;
                    if normalized { x / 255.0 } else { x }
                }
                _ => {
                    let x = b[0] as i8 as f32;
                    if normalized { (x / 127.0).max(-1.0) } else { x }
                }
            };
        }
        out.push(Vec4::from_array(v));
    }
    Ok(out)
}

/// Линейный цвет 0..1 — в байт sRGB.
fn srgb(c: f32) -> u8 {
    let c = c.clamp(0.0, 1.0);
    let s = if c <= 0.003_130_8 { c * 12.92 } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 };
    (s * 255.0).round() as u8
}

/// Разобранный JSON — ровно столько, сколько нужно для glTF.
#[derive(Debug, PartialEq)]
enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    fn parse(text: &str) -> Result<Json, String> {
        let mut p = Parser { s: text.as_bytes(), at: 0 };
        let v = p.value(0)?;
        p.space();
        if p.at != p.s.len() {
            return Err(format!("лишнее после JSON на {}", p.at));
        }
        Ok(v)
    }

    fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    fn at(&self, i: usize) -> Option<&Json> {
        self.items().get(i)
    }

    fn items(&self) -> &[Json] {
        match self {
            Json::Arr(v) => v,
            _ => &[],
        }
    }

    fn as_f32(&self) -> Option<f32> {
        match self {
            Json::Num(n) => Some(*n as f32),
            _ => None,
        }
    }

    fn as_usize(&self) -> Option<usize> {
        match self {
            Json::Num(n) if *n >= 0.0 && n.fract() == 0.0 => Some(*n as usize),
            _ => None,
        }
    }

    fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }
}

struct Parser<'a> {
    s: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn space(&mut self) {
        while self.at < self.s.len() && self.s[self.at].is_ascii_whitespace() {
            self.at += 1;
        }
    }

    fn expect(&mut self, c: u8) -> Result<(), String> {
        self.space();
        if self.s.get(self.at) == Some(&c) {
            self.at += 1;
            Ok(())
        } else {
            Err(format!("ждали «{}» на {}", c as char, self.at))
        }
    }

    fn value(&mut self, depth: u32) -> Result<Json, String> {
        if depth > 128 {
            return Err("JSON слишком глубокий".into());
        }
        self.space();
        match self.s.get(self.at) {
            Some(b'{') => {
                self.at += 1;
                let mut fields = Vec::new();
                self.space();
                if self.s.get(self.at) == Some(&b'}') {
                    self.at += 1;
                    return Ok(Json::Obj(fields));
                }
                loop {
                    self.space();
                    let key = self.string()?;
                    self.expect(b':')?;
                    fields.push((key, self.value(depth + 1)?));
                    self.space();
                    match self.s.get(self.at) {
                        Some(b',') => self.at += 1,
                        Some(b'}') => {
                            self.at += 1;
                            return Ok(Json::Obj(fields));
                        }
                        _ => return Err(format!("ждали «,» или «}}» на {}", self.at)),
                    }
                }
            }
            Some(b'[') => {
                self.at += 1;
                let mut items = Vec::new();
                self.space();
                if self.s.get(self.at) == Some(&b']') {
                    self.at += 1;
                    return Ok(Json::Arr(items));
                }
                loop {
                    items.push(self.value(depth + 1)?);
                    self.space();
                    match self.s.get(self.at) {
                        Some(b',') => self.at += 1,
                        Some(b']') => {
                            self.at += 1;
                            return Ok(Json::Arr(items));
                        }
                        _ => return Err(format!("ждали «,» или «]» на {}", self.at)),
                    }
                }
            }
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') => self.word("true", Json::Bool(true)),
            Some(b'f') => self.word("false", Json::Bool(false)),
            Some(b'n') => self.word("null", Json::Null),
            Some(_) => self.number(),
            None => Err("JSON оборвался".into()),
        }
    }

    fn word(&mut self, w: &str, v: Json) -> Result<Json, String> {
        if self.s[self.at..].starts_with(w.as_bytes()) {
            self.at += w.len();
            Ok(v)
        } else {
            Err(format!("непонятное слово на {}", self.at))
        }
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.at;
        while self.at < self.s.len() && matches!(self.s[self.at], b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9') {
            self.at += 1;
        }
        let text = std::str::from_utf8(&self.s[start..self.at]).map_err(|e| e.to_string())?;
        text.parse::<f64>().map(Json::Num).map_err(|_| format!("не число «{text}» на {start}"))
    }

    fn string(&mut self) -> Result<String, String> {
        if self.s.get(self.at) != Some(&b'"') {
            return Err(format!("ждали строку на {}", self.at));
        }
        self.at += 1;
        let mut out = Vec::new();
        loop {
            let c = *self.s.get(self.at).ok_or("строка не закрыта")?;
            self.at += 1;
            match c {
                b'"' => break,
                b'\\' => {
                    let e = *self.s.get(self.at).ok_or("строка не закрыта")?;
                    self.at += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b't' => out.push(b'\t'),
                        b'r' => out.push(b'\r'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'u' => {
                            let mut code = self.hex4()?;
                            // Суррогатная пара — один символ.
                            if (0xD800..0xDC00).contains(&code) && self.s[self.at..].starts_with(b"\\u") {
                                self.at += 2;
                                let low = self.hex4()?;
                                code = 0x10000 + ((code - 0xD800) << 10) + (low.wrapping_sub(0xDC00) & 0x3FF);
                            }
                            let ch = char::from_u32(code).unwrap_or('\u{FFFD}');
                            out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
                        }
                        other => out.push(other),
                    }
                }
                _ => out.push(c),
            }
        }
        String::from_utf8(out).map_err(|e| e.to_string())
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let h = self.s.get(self.at..self.at + 4).ok_or("обрезан \\u")?;
        self.at += 4;
        u32::from_str_radix(std::str::from_utf8(h).map_err(|e| e.to_string())?, 16).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn json_parses_what_gltf_needs() {
        let j = Json::parse(r#"{"a": [1, -2.5e-1, true, null], "имя": "изба", "o": {}}"#).unwrap();
        assert_eq!(j.get("a").unwrap().items().len(), 4);
        assert_eq!(j.get("a").unwrap().at(1).unwrap().as_f32(), Some(-0.25));
        assert_eq!(j.get("имя").unwrap().as_str(), Some("изба"));
        assert!(Json::parse("{\"a\": }").is_err());
        assert!(Json::parse("[1, 2").is_err());
    }

    /// Самодельный .glb: один треугольник, узел сдвинут и масштабирован, цвет — из
    /// материала.
    pub(crate) fn triangle_glb() -> Vec<u8> {
        let mut bin = Vec::new();
        for v in [0.0f32, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0] {
            bin.extend(v.to_le_bytes());
        }
        let json = r#"{"asset":{"version":"2.0"},"scene":0,"scenes":[{"nodes":[0]}],
            "nodes":[{"mesh":0,"translation":[10,0,0],"scale":[2,2,2]}],
            "meshes":[{"primitives":[{"attributes":{"POSITION":0},"material":0}]}],
            "materials":[{"pbrMetallicRoughness":{"baseColorFactor":[0.2159,1,0,1]}}],
            "accessors":[{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3"}],
            "bufferViews":[{"buffer":0,"byteLength":36}],"buffers":[{"byteLength":36}]}"#;
        let mut json = json.as_bytes().to_vec();
        while !json.len().is_multiple_of(4) {
            json.push(b' ');
        }
        let mut out = Vec::new();
        out.extend(0x4654_6C67u32.to_le_bytes());
        out.extend(2u32.to_le_bytes());
        out.extend(((12 + 8 + json.len() + 8 + bin.len()) as u32).to_le_bytes());
        out.extend((json.len() as u32).to_le_bytes());
        out.extend(0x4E4F_534Au32.to_le_bytes());
        out.extend(json);
        out.extend((bin.len() as u32).to_le_bytes());
        out.extend(0x004E_4942u32.to_le_bytes());
        out.extend(bin);
        out
    }

    #[test]
    fn glb_triangle_with_node_transform_and_material_color() {
        let mesh = load(&triangle_glb()).unwrap();
        assert_eq!(mesh.indices.len(), 3);
        assert_eq!(mesh.vertices[1].pos, [12.0, 0.0, 0.0]);
        assert_eq!(mesh.vertices[2].pos, [10.0, 0.0, 2.0]);
        // Нормаль плоского треугольника в плоскости XZ — вниз (−Y: обход по часовой сверху).
        assert_eq!(mesh.vertices[0].normal[1], -127);
        // Линейные 0,2159 → sRGB 128.
        assert_eq!(mesh.vertices[0].color, [128, 255, 0, 255]);
        assert!(load(b"not a glb").is_err());
    }

    #[test]
    fn real_blender_export_loads() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/models/props/изба.glb");
        let Ok(bytes) = std::fs::read(&path) else { return };
        let mesh = load(&bytes).unwrap();
        let (lo, hi) = mesh.vertices.iter().fold((Vec3::splat(f32::MAX), Vec3::splat(f32::MIN)), |(lo, hi), v| {
            let p = Vec3::from_array(v.pos);
            (lo.min(p), hi.max(p))
        });
        println!("изба: {} вершин, от {lo} до {hi}", mesh.vertices.len());
        // Стоит на земле (ступени крыльца чуть в земле), высотой в несколько метров.
        assert!(lo.y > -0.2 && lo.y < 0.01 && hi.y > 4.0 && hi.y < 8.0);
    }
}
