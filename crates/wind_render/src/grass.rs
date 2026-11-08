//! Трава карточками — «GrassV1» хозяина (`grass_lab.blend`, сцена `GrassV1_Pads`).
//!
//! Шесть кусков травы (`data/models/grass/<кусок>.glb`): карточка, согнутая пополам
//! (30°) и чуть наклонённая назад, 8 треугольников; картинка — из общего атласа
//! `data/models/grass/atlas.png` (рисованные стебли с контуром, прозрачность — отсечение).
//! Нормаль у всех вершин — вверх: кусок освещён как земля под ним.
//!
//! Цвета — свои, из атласа хозяина (осенняя палитра), без перекраски. Высота — как у
//! зелёной травы RWR2 (ref-6 у забора, ref-4 у домов): куски в 1,6 раза выше, чем на
//! площадках, и реже.
//!
//! Каждый кусок повёрнут лицом к камере (с разбросом ±0,45 рад) — верхушки
//! торчат вверх по экрану, картинка читается, ребром и изнанкой куски не встают. Тень
//! отбрасывают только полосы у заборов, стен и стволов; в поле тени от каждого куска
//! покрывали землю тёмными «ветками».
//!
//! Расстановка — правила площадок `build_pads` (`analyze/blender/grass_v1.py`), переложенные
//! на карту:
//! - густота — плавное поле по шуму: в середине пятен сплошь (8 кусков/м² на «Сухой
//!   траве», 7 — на «Траве»), к краю реже, между пятнами — редкие одиночные куски
//!   и кучки, так что трава нигде не обрывается одеялом;
//! - «Сухая трава» — низкая, местами с примесью средней и кустиков; «Трава» — луг, кое-где
//!   высокая (стелс);
//! - пятна бурьяна (кучки сорняков и кустиков) на любой траве;
//! - вдоль заборов, стен домов и стен из клеток — полоса (шаг 0,35 м), у стволов — кольцо;
//! - у края травы к дороге — редкие кучки.
//!
//! Файлы травы перечитываются на лету (`GrassLibrary`): выгрузил из Blender — трава в игре
//! и в редакторе сменилась без перезапуска.
//!
//! Пятна — от плавного шума по карте, всё — от хеша: у всех игроков трава одна и та же.

use glam::Vec2;
use wind_sim::map::{CHUNK, Map};
use wind_sim::mapfile::Prop;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// Куски травы — порядок как в атласе и в папке.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Low,
    Mid,
    Tall,
    Khaki,
    Weeds,
    Clump,
}

pub const KINDS: [Kind; 6] = [Kind::Low, Kind::Mid, Kind::Tall, Kind::Khaki, Kind::Weeds, Kind::Clump];

impl Kind {
    pub fn file(self) -> &'static str {
        match self {
            Kind::Low => "low",
            Kind::Mid => "mid",
            Kind::Tall => "tall",
            Kind::Khaki => "khaki",
            Kind::Weeds => "weeds",
            Kind::Clump => "clump",
        }
    }

    /// Разброс размера: `PIECE_SCALE` из `grass_v1.py`, умноженный на [`GROW`].
    fn scale(self) -> (f32, f32) {
        let (a, b) = match self {
            Kind::Low => (0.65, 0.85),
            Kind::Mid => (0.9, 1.1),
            Kind::Tall => (1.2, 1.5),
            Kind::Khaki => (1.15, 1.45),
            Kind::Weeds => (0.95, 1.25),
            Kind::Clump => (0.85, 1.1),
        };
        (a * GROW, b * GROW)
    }
}

/// Насколько трава выше, чем на площадках в Blender: зелёная трава у RWR2 (ref-6 у забора,
/// ref-4 у домов) — по колено и выше, у забора — почти до верха столбов. Куски 0,63 м ×
/// 1,6 дают 0,65–1,5 м.
const GROW: f32 = 1.6;

/// Вершина куска: положение и место в атласе.
#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct CardVertex {
    pub pos: [f32; 3],
    pub uv: [f32; 2],
}

/// Кусок травы на карте — так и уходит на GPU (32 байта).
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Piece {
    pub pos: [f32; 3],
    /// Разброс поворота от «лицом к камере», рад (см. `vs_grass`).
    pub angle: f32,
    pub scale: f32,
    /// Яркость (разброс ±7 %, как `jitter` материала в Blender).
    pub shade: f32,
    pub kind: u32,
    /// Признаки: [`CASTS_SHADOW`].
    pub flags: u32,
}

/// Кусок отбрасывает тень — только полосы у заборов, стен и стволов (как у RWR2: в поле
/// тени от травы нет, у забора — зубчатая тень).
pub const CASTS_SHADOW: u32 = 1;

/// Разброс поворота кусков от «лицом к камере», рад.
const TURN: f32 = 0.45;

/// Трава карты, готовая к отправке на GPU: куски по видам, внутри вида — по чанкам.
#[derive(Default)]
pub struct GrassUpload {
    pub pieces: Vec<Piece>,
    /// Для каждого вида и чанка: (первый кусок, сколько). Индекс — вид × чанков + чанк.
    pub ranges: Vec<(u32, u32)>,
    pub chunks: usize,
    /// Густота травы по клеткам (0..255, 255 — сплошная): под густой травой земля
    /// подкрашивается в её тон, как сплошная масса травы у RWR2, — без неё между кусками
    /// просвечивает светлая земля и трава рябит. Ширина и высота — как у карты.
    pub density: (u32, u32, Vec<u8>),
}

/// Сколько кусков в клетке — «сплошная» трава.
const FULL: f32 = 6.0;

/// Файлы травы: куски и атлас.
pub struct GrassAssets {
    /// Меш каждого вида по порядку [`KINDS`]: вершины и треугольники.
    pub meshes: Vec<(Vec<CardVertex>, Vec<u32>)>,
    /// Атлас RGBA8 (sRGB): ширина, высота, тексели.
    pub atlas: (u32, u32, Vec<u8>),
}

impl GrassAssets {
    /// Из папки `data/models/grass`.
    pub fn load(dir: &Path) -> Result<GrassAssets, String> {
        let meshes = KINDS
            .iter()
            .map(|k| {
                let path = dir.join(format!("{}.glb", k.file()));
                let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                crate::gltf::load_cards(&bytes).map_err(|e| format!("{}: {e}", path.display()))
            })
            .collect::<Result<Vec<_>, String>>()?;
        let path = dir.join("atlas.png");
        let atlas = read_rgba(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(GrassAssets { meshes, atlas })
    }
}

/// Папка травы, перечитываемая на лету: раз в полсекунды сверяет время изменения файлов
/// и, если что-то поменялось, читает всё заново (недописанный файл не прочитается — тогда
/// прочитаем, когда его допишут и время сменится).
#[derive(Default)]
pub struct GrassLibrary {
    dir: Option<PathBuf>,
    stamp: Option<Vec<Option<SystemTime>>>,
    last_poll: Option<Instant>,
}

impl GrassLibrary {
    pub fn new(dir: PathBuf) -> Self {
        GrassLibrary { dir: Some(dir), ..Default::default() }
    }

    fn files(dir: &Path) -> Vec<PathBuf> {
        let mut files: Vec<PathBuf> = KINDS.iter().map(|k| dir.join(format!("{}.glb", k.file()))).collect();
        files.push(dir.join("atlas.png"));
        files
    }

    /// Новые файлы травы, если они поменялись с прошлого раза (в первый раз — сразу).
    pub fn poll(&mut self) -> Option<Result<GrassAssets, String>> {
        let dir = self.dir.as_ref()?;
        if self.last_poll.is_some_and(|t| t.elapsed() < Duration::from_millis(500)) {
            return None;
        }
        self.last_poll = Some(Instant::now());
        let stamp: Vec<Option<SystemTime>> = Self::files(dir).iter().map(|f| std::fs::metadata(f).and_then(|m| m.modified()).ok()).collect();
        if self.stamp.as_ref() == Some(&stamp) {
            return None;
        }
        self.stamp = Some(stamp);
        Some(GrassAssets::load(dir))
    }
}

fn read_rgba(path: &Path) -> Result<(u32, u32, Vec<u8>), String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut decoder = png::Decoder::new(std::io::BufReader::new(file));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buf = vec![0; reader.output_buffer_size().ok_or("картинка слишком большая")?];
    let info = reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
    let (w, h) = (info.width, info.height);
    let px = (w * h) as usize;
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..px * 4].to_vec(),
        png::ColorType::Rgb => buf[..px * 3].chunks(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buf[..px * 2].chunks(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Grayscale => buf[..px].iter().flat_map(|&c| [c, c, c, 255]).collect(),
        other => return Err(format!("цвет {other:?} не поддерживается")),
    };
    Ok((w, h, rgba))
}

/// Мип-уровни атласа для отсечения по альфе: цвет — среднее четырёх, альфа подобрана так,
/// чтобы доля непрозрачного (альфа > 0,5) оставалась как на полном размере — иначе тонкие
/// стебли вдали тают.
pub fn atlas_mips(w: u32, h: u32, rgba: &[u8]) -> Vec<(u32, u32, Vec<u8>)> {
    let coverage = |t: &[u8], scale: f32| t.chunks(4).filter(|p| p[3] as f32 * scale > 127.5).count() as f32 / (t.len() / 4).max(1) as f32;
    let target = coverage(rgba, 1.0);
    let mut levels = vec![(w, h, rgba.to_vec())];
    while let Some((lw, lh, prev)) = levels.last().filter(|(lw, lh, _)| *lw > 1 || *lh > 1) {
        let (nw, nh) = ((lw / 2).max(1), (lh / 2).max(1));
        let mut next = vec![0u8; (nw * nh * 4) as usize];
        for y in 0..nh {
            for x in 0..nw {
                for c in 0..4 {
                    let mut sum = 0u32;
                    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                        let (sx, sy) = ((x * 2 + dx).min(lw - 1), (y * 2 + dy).min(lh - 1));
                        sum += prev[((sy * lw + sx) * 4 + c) as usize] as u32;
                    }
                    next[((y * nw + x) * 4 + c) as usize] = (sum / 4) as u8;
                }
            }
        }
        // Подбор множителя альфы делением пополам.
        let (mut lo, mut hi) = (0.5f32, 4.0f32);
        for _ in 0..12 {
            let mid = (lo + hi) * 0.5;
            if coverage(&next, mid) < target {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        let scale = (lo + hi) * 0.5;
        for p in next.chunks_mut(4) {
            p[3] = (p[3] as f32 * scale).min(255.0) as u8;
        }
        levels.push((nw, nh, next));
    }
    levels
}

// --- Расстановка ------------------------------------------------------------------------

/// Основание предмета в его координатах: (min x, min z, max x, max z), м — что у земли
/// (см. `PropLibrary::footprint`).
pub type Footprint = [f32; 4];

/// Целочисленный хеш → 0..1.
fn hash(a: u32, b: u32, c: u32) -> f32 {
    let mut h = a.wrapping_mul(0x9E37_79B1) ^ b.wrapping_mul(0x85EB_CA77) ^ c.wrapping_mul(0xC2B2_AE3D);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2C1B_3C6D);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297A_2D39);
    h ^= h >> 15;
    (h >> 8) as f32 / 16_777_216.0
}

/// Плавный шум 0..1 с ячейкой `cell` м (значения в узлах — от хеша, между ними — сглаженно).
fn smooth_noise(p: Vec2, cell: f32, seed: u32) -> f32 {
    let q = p / cell;
    let (i, f) = (q.floor(), q - q.floor());
    let u = f * f * (Vec2::splat(3.0) - 2.0 * f);
    let at = |dx: i32, dy: i32| hash((i.x as i32 + dx) as u32, (i.y as i32 + dy) as u32, seed);
    let a = at(0, 0) + (at(1, 0) - at(0, 0)) * u.x;
    let b = at(0, 1) + (at(1, 1) - at(0, 1)) * u.x;
    a + (b - a) * u.y
}

/// Два слоя шума — пятна неровнее.
fn patches(p: Vec2, cell: f32, seed: u32) -> f32 {
    smooth_noise(p, cell, seed) * 0.7 + smooth_noise(p, cell * 0.37, seed + 1) * 0.3
}

/// Доля кусков между пятнами — редкие одиночные, чтобы трава не обрывалась голой землёй.
const SPARSE: f32 = 0.04;

fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Веса видов (как словари `weights` в `build_pads`).
type Weights = &'static [(Kind, f32)];
const LOW_COVER: Weights = &[(Kind::Low, 8.0), (Kind::Mid, 1.0)];
const DRY_MIXED: Weights = &[(Kind::Low, 5.0), (Kind::Mid, 3.0), (Kind::Clump, 1.0)];
const MEADOW: Weights = &[(Kind::Mid, 5.0), (Kind::Low, 4.0), (Kind::Khaki, 1.0)];
const MEADOW_TALL: Weights = &[(Kind::Tall, 2.0), (Kind::Khaki, 1.0)];
const TALL: Weights = &[(Kind::Tall, 5.0), (Kind::Khaki, 3.0), (Kind::Mid, 1.0), (Kind::Weeds, 1.0)];
const WEEDS: Weights = &[(Kind::Weeds, 4.0), (Kind::Clump, 3.0), (Kind::Tall, 1.0), (Kind::Low, 2.0)];
const EDGE: Weights = &[(Kind::Low, 2.0), (Kind::Mid, 1.0)];
const ALONG: Weights = &[(Kind::Tall, 3.0), (Kind::Mid, 3.0), (Kind::Khaki, 2.0), (Kind::Weeds, 1.0), (Kind::Low, 1.0)];
const TRUNK: Weights = &[(Kind::Mid, 2.0), (Kind::Tall, 2.0), (Kind::Weeds, 1.0)];

fn pick(weights: Weights, r: f32) -> Kind {
    let total: f32 = weights.iter().map(|w| w.1).sum();
    let mut x = r * total;
    for &(k, w) in weights {
        if x < w {
            return k;
        }
        x -= w;
    }
    weights[weights.len() - 1].0
}

/// Трава ли в точке: покрытие «Трава» или «Сухая трава», не стена и не вода.
fn grass_at(map: &Map, p: Vec2) -> Option<u8> {
    let (x, y) = Map::cell_of(p);
    let i = map.index(x, y)?;
    let g = map.ground[i];
    (g <= 1 && map.solid[i] == 0 && map.water_depth(p) <= 0.02).then_some(g)
}

struct Placer<'a> {
    map: &'a Map,
    out: Vec<Piece>,
}

impl Placer<'_> {
    /// Кусок вида из `weights` в точке `p`; `seed` — откуда брать случайности.
    fn put(&mut self, p: Vec2, weights: Weights, seed: (u32, u32, u32), flags: u32) {
        if grass_at(self.map, p).is_none() {
            return;
        }
        let h = |k: u32| hash(seed.0, seed.1, seed.2.wrapping_mul(8).wrapping_add(k));
        let angle = (h(4) - 0.5) * 2.0 * TURN;
        let kind = pick(weights, h(1));
        let (s0, s1) = kind.scale();
        self.out.push(Piece {
            pos: [p.x, self.map.height_at(p), p.y],
            angle,
            scale: s0 + (s1 - s0) * h(2),
            shade: 0.93 + 0.14 * h(3),
            kind: kind as u32,
            flags,
        });
    }

    /// Кучка из `n` кусков в круге радиуса `r`.
    fn clump(&mut self, c: Vec2, r: f32, n: u32, weights: Weights, seed: (u32, u32, u32)) {
        for k in 0..n {
            let s = (seed.0, seed.1, seed.2.wrapping_mul(32).wrapping_add(k));
            let a = hash(s.0, s.1, s.2 ^ 0x51) * std::f32::consts::TAU;
            let rr = r * hash(s.0, s.1, s.2 ^ 0x52).sqrt();
            self.put(c + Vec2::new(a.cos(), a.sin()) * rr, weights, s, 0);
        }
    }

    /// Полоса вдоль отрезка a→b (`along` в `build_pads`): шаг 0,35 м, 90 % мест, отступ от
    /// линии `offset`; `both` — по обе стороны, иначе — слева по ходу (наружу при обходе
    /// по часовой).
    fn along(&mut self, a: Vec2, b: Vec2, offset: (f32, f32), both: bool, seed: u32) {
        let d = b - a;
        let len = d.length();
        if len < 1e-3 {
            return;
        }
        let dir = d / len;
        let nrm = Vec2::new(-dir.y, dir.x);
        let n = (len / 0.5) as u32;
        for k in 0..=n {
            for side in [1.0f32, -1.0] {
                if side < 0.0 && !both {
                    continue;
                }
                let s = (seed, k, if side > 0.0 { 1 } else { 2 });
                let h = |j: u32| hash(s.0, s.1, s.2 * 16 + j);
                if h(0) > 0.9 {
                    continue;
                }
                let t = len * k as f32 / n.max(1) as f32 + (h(1) - 0.5) * 0.3;
                let p = a + dir * t + nrm * side * (offset.0 + (offset.1 - offset.0) * h(2));
                self.put(p, ALONG, (s.0, s.1, s.2 + 100), CASTS_SHADOW);
            }
        }
    }
}

/// Трава всей карты.
pub fn place(map: &Map, props: &[Prop], footprint: impl Fn(&str) -> Option<Footprint>) -> Vec<Piece> {
    let mut pl = Placer { map, out: Vec::new() };

    // У предметов: вдоль краёв основания (дом, забор, колодец), у ствола — кольцо.
    for (n, prop) in props.iter().enumerate() {
        let Some([x0, z0, x1, z1]) = footprint(&prop.model) else { continue };
        let (sin, cos) = prop.angle.sin_cos();
        let s = prop.scale;
        let world = |x: f32, z: f32| prop.at() + Vec2::new(x * cos - z * sin, x * sin + z * cos) * s;
        let (w, d) = ((x1 - x0) * s, (z1 - z0) * s);
        let n = n as u32 + 1;
        if w.max(d) < 0.8 {
            // Ствол, столб: пять кусков кольцом, лицом по касательной.
            let c = world((x0 + x1) * 0.5, (z0 + z1) * 0.5);
            let r = w.max(d) * 0.5 + 0.3;
            for k in 0..5 {
                let a = (k as f32 + 0.8 * hash(n, k, 1)) / 5.0 * std::f32::consts::TAU;
                pl.put(c + Vec2::new(a.cos(), a.sin()) * r, TRUNK, (n, k, 2), CASTS_SHADOW);
            }
        } else if w.min(d) < 0.5 {
            // Забор: полоса с обеих сторон по длинной оси, с запасом 0,3 м на концах.
            let (a, b) = if w >= d {
                (world(x0 - 0.3 / s, (z0 + z1) * 0.5), world(x1 + 0.3 / s, (z0 + z1) * 0.5))
            } else {
                (world((x0 + x1) * 0.5, z0 - 0.3 / s), world((x0 + x1) * 0.5, z1 + 0.3 / s))
            };
            pl.along(a, b, (0.1, 0.5), true, n * 8);
        } else {
            // Дом, сарай: по периметру наружу, обход по часовой (в координатах карты).
            let corners = [world(x1, z0), world(x0, z0), world(x0, z1), world(x1, z1)];
            let cw = (corners[1] - corners[0]).perp_dot(corners[2] - corners[1]) > 0.0;
            for k in 0..4 {
                let (a, b) = (corners[k], corners[(k + 1) % 4]);
                let (a, b) = if cw { (b, a) } else { (a, b) };
                pl.along(a, b, (0.1, 0.4), false, n * 8 + 1 + k as u32);
            }
        }
    }

    // По клеткам.
    let dirs = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    for y in 0..map.height as i32 {
        for x in 0..map.width as i32 {
            let i = y as usize * map.width as usize + x as usize;
            let g = map.ground[i];
            if g > 1 || map.solid[i] != 0 {
                continue;
            }
            let (ux, uy) = (x as u32, y as u32);
            let cell = Vec2::new(x as f32, y as f32);

            // Стены из клеток и край травы у дороги.
            for (k, (dx, dy)) in dirs.into_iter().enumerate() {
                let Some(j) = map.index(x + dx, y + dy) else { continue };
                let k = k as u32;
                let (a, b) = match (dx, dy) {
                    (1, 0) => (cell + Vec2::new(1.0, 0.0), cell + Vec2::new(1.0, 1.0)),
                    (-1, 0) => (cell + Vec2::new(0.0, 1.0), cell),
                    (0, 1) => (cell + Vec2::new(1.0, 1.0), cell + Vec2::new(0.0, 1.0)),
                    _ => (cell, cell + Vec2::new(1.0, 0.0)),
                };
                if map.solid[j] != 0 {
                    // Слева по ходу a→b — внутрь клетки травы.
                    pl.along(a, b, (0.1, 0.4), false, 0x8000_0000 ^ ((uy * 4096 + ux) * 4 + k));
                } else if map.ground[j] > 1 && hash(ux, uy, 300 + k) < 0.15 {
                    let mid = (a + b) * 0.5 + Vec2::new(-(b - a).y, (b - a).x) * 0.35;
                    pl.clump(mid, 0.3, 2 + (hash(ux, uy, 310 + k) * 1.99) as u32, EDGE, (ux, uy, 320 + k));
                }
            }

            // Бурьян — кучками по своим пятнам; густота плавно спадает к краю пятна.
            let center = cell + Vec2::splat(0.5);
            let weeds = smoothstep(0.66, 0.8, patches(center, 11.0, 7));
            if hash(ux, uy, 400) < weeds * 0.4 {
                let n = 3 + (hash(ux, uy, 401) * 4.99) as u32;
                pl.clump(center, 0.65, n, WEEDS, (ux, uy, 402));
                continue;
            }

            // Основной покров: густота — плавное поле по шуму, от сплошной травы в середине
            // пятен до редких одиночных кусков между ними; голой «Травы» почти нет.
            // Куски крупнее, чем на площадках (`GROW`), — и реже: 16 и 13 на м², делённые на
            // ~2, иначе высокая трава сливается в войлок.
            let max_density = if g == 1 { 8 } else { 7 };
            for k in 0..max_density {
                let p = cell + Vec2::new(hash(ux, uy, 500 + k * 2), hash(ux, uy, 501 + k * 2));
                let share = smoothstep(0.3, 0.64, patches(p, 9.0, 3)).max(SPARSE);
                if hash(ux, uy, 600 + k) >= share {
                    continue;
                }
                let weights = if g == 1 {
                    // Сухая трава: низкая, местами — с примесью средней и кустиков.
                    if patches(p, 6.0, 11) > 0.62 { DRY_MIXED } else { LOW_COVER }
                } else if patches(p, 14.0, 5) > 0.62 + 0.08 * hash(ux, uy, 650 + k) {
                    TALL
                } else {
                    MEADOW
                };
                pl.put(p, weights, (ux, uy, 600 + k), 0);
            }
            // Кучки по всей траве — разбивают ровный покров и пустые места.
            if hash(ux, uy, 800) < 0.05 {
                let p = cell + Vec2::new(hash(ux, uy, 802), hash(ux, uy, 803));
                pl.clump(p, 0.35, 2 + (hash(ux, uy, 801) * 2.99) as u32, if g == 1 { EDGE } else { MEADOW_TALL }, (ux, uy, 804));
            }
        }
    }
    pl.out
}

/// Разложить куски по видам и чанкам для отрисовки только видимых чанков.
pub fn upload(map: &Map, mut pieces: Vec<Piece>) -> GrassUpload {
    let cx = map.chunks_x() as usize;
    let chunks = map.chunk_revisions.len();
    let chunk_of = |p: &Piece| {
        let x = (p.pos[0].max(0.0) as u32 / CHUNK) as usize;
        let y = (p.pos[2].max(0.0) as u32 / CHUNK) as usize;
        (y * cx + x).min(chunks.saturating_sub(1))
    };
    let (w, h) = (map.width, map.height);
    let mut count = vec![0f32; (w * h) as usize];
    for p in &pieces {
        let (x, y) = Map::cell_of(Vec2::new(p.pos[0], p.pos[2]));
        if let Some(i) = map.index(x, y) {
            count[i] += 1.0;
        }
    }
    let density = count.iter().map(|&c| (c / FULL * 255.0).min(255.0) as u8).collect();
    pieces.sort_by_key(|p| (p.kind, chunk_of(p)));
    let mut ranges = vec![(0u32, 0u32); KINDS.len() * chunks];
    for (i, p) in pieces.iter().enumerate() {
        let r = &mut ranges[p.kind as usize * chunks + chunk_of(p)];
        if r.1 == 0 {
            r.0 = i as u32;
        }
        r.1 += 1;
    }
    GrassUpload { pieces, ranges, chunks, density: (w, h, density) }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Поле 64×64: слева «Трава», справа «Сухая трава», дорога (земля) поперёк, стена из
    /// клеток и забор.
    fn sample() -> (Map, Vec<Prop>) {
        let mut map = Map::new(64, 64);
        for y in 0..64 {
            for x in 32..64 {
                map.ground[y * 64 + x] = 1;
            }
            if (30..33).contains(&y) {
                for x in 0..64 {
                    map.ground[y * 64 + x] = 2;
                }
            }
        }
        for x in 5..12 {
            map.set(x, 50, 255);
        }
        let fence = Prop { model: "забор".into(), pos: (20.0, 12.0), angle: 0.4, scale: 1.0 };
        (map, vec![fence])
    }

    fn footprint(name: &str) -> Option<Footprint> {
        (name == "забор").then_some([-2.0, -0.05, 2.0, 0.05])
    }

    #[test]
    fn grass_stands_only_on_grass_and_lines_fences_and_walls() {
        let (map, props) = sample();
        let pieces = place(&map, &props, footprint);
        assert!(!pieces.is_empty());
        for p in &pieces {
            let (x, y) = Map::cell_of(Vec2::new(p.pos[0], p.pos[2]));
            let i = map.index(x, y).expect("в пределах карты");
            assert!(map.ground[i] <= 1 && map.solid[i] == 0, "трава не на траве: {:?}", p.pos);
        }
        let at = |p: &Piece| Vec2::new(p.pos[0], p.pos[2]);
        let near_fence = pieces.iter().filter(|p| at(p).distance(Vec2::new(20.0, 12.0)) < 2.6).count();
        assert!(near_fence >= 15, "у забора мало травы: {near_fence}");
        let near_wall = pieces.iter().filter(|p| (at(p).y - 50.5).abs() < 1.2 && (5.0..12.0).contains(&at(p).x)).count();
        assert!(near_wall >= 15, "у стены мало травы: {near_wall}");
        // Густота плавно меняется: есть и сплошные места, и редкие, а не одеяло одной густоты.
        let mut per_cell = vec![0u32; 64 * 64];
        for p in &pieces {
            let (x, y) = Map::cell_of(at(p));
            per_cell[y as usize * 64 + x as usize] += 1;
        }
        let grass_cells: Vec<u32> = (0..64 * 64).filter(|&i| map.ground[i] <= 1 && map.solid[i] == 0).map(|i| per_cell[i]).collect();
        let dense = grass_cells.iter().filter(|&&c| c >= 5).count();
        let sparse = grass_cells.iter().filter(|&&c| (1..=2).contains(&c)).count();
        let bare = grass_cells.iter().filter(|&&c| c == 0).count();
        assert!(dense > 100 && sparse > 100, "густота не меняется: густых {dense}, редких {sparse}");
        assert!(bare * 2 < grass_cells.len(), "слишком много голой травы: {bare} из {}", grass_cells.len());
    }

    #[test]
    fn placement_is_the_same_every_time_and_ranges_cover_all() {
        let (map, props) = sample();
        let a = place(&map, &props, footprint);
        assert_eq!(a, place(&map, &props, footprint));
        let up = upload(&map, a.clone());
        assert_eq!(up.pieces.len(), a.len());
        assert_eq!(up.ranges.iter().map(|r| r.1 as usize).sum::<usize>(), a.len());
        for (i, &(first, count)) in up.ranges.iter().enumerate() {
            for p in &up.pieces[first as usize..(first + count) as usize] {
                assert_eq!(p.kind as usize, i / up.chunks);
            }
        }
    }

    #[test]
    fn grass_library_reloads_when_files_change() {
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/models/grass");
        let dir = std::env::temp_dir().join("wind_grass_reload_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for f in GrassLibrary::files(&src) {
            std::fs::copy(&f, dir.join(f.file_name().unwrap())).unwrap();
        }
        let mut lib = GrassLibrary::new(dir.clone());
        assert!(matches!(lib.poll(), Some(Ok(_))), "в первый раз читает сразу");
        lib.last_poll = None;
        assert!(lib.poll().is_none(), "ничего не менялось");
        // Хозяин перевыгрузил кусок из Blender.
        let glb = dir.join("low.glb");
        let file = std::fs::OpenOptions::new().write(true).open(&glb).unwrap();
        file.set_modified(SystemTime::now() + Duration::from_secs(5)).unwrap();
        lib.last_poll = None;
        assert!(matches!(lib.poll(), Some(Ok(_))), "заметил новый файл");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn grass_files_load() {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data/models/grass");
        let assets = GrassAssets::load(&dir).unwrap();
        assert_eq!(assets.meshes.len(), KINDS.len());
        for (v, i) in &assets.meshes {
            assert!(!v.is_empty() && i.len() % 3 == 0);
        }
        let (w, h, px) = &assets.atlas;
        assert_eq!(px.len(), (w * h * 4) as usize);
        let mips = atlas_mips(64, 64, &px[..64 * 64 * 4]);
        assert_eq!(mips.len(), 7);
    }
}
