//! Редкие пучки травы — приём RWR/RWR2: трава в основном нарисована на земле
//! (`ground_color` в `world.wgsl`), а отдельные высокие пучки стоят там, где трава в жизни
//! не выкошена: у заборов, у стен и углов домов, у стволов, по краю травы у дорог и
//! изредка кучками в поле. Ковра нет — на карте сотни пучков, а не миллионы травинок.
//!
//! Пучок — маленькая модель (`models::tuft`): веер плоских стеблей, свет — цветом вершин.
//! Расстановка не случайная от запуска к запуску: всё — от хеша клетки, так что у всех
//! игроков и в редакторе пучки одни и те же.

use glam::Vec2;
use wind_sim::map::Map;
use wind_sim::mapfile::Prop;

/// Один пучок на земле.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tuft {
    pub pos: Vec2,
    /// Поворот вокруг вертикали, рад.
    pub angle: f32,
    pub scale: f32,
    /// Цвет травы (sRGB, множитель цвета вершин).
    pub tint: [f32; 3],
}

/// Основание предмета в его координатах: (min x, min z, max x, max z), м — что у земли
/// (см. `PropLibrary::footprint`).
pub type Footprint = [f32; 4];

/// Шаг вдоль заборов и стен, м.
const STEP: f32 = 0.7;
/// Доля мест у заборов и стен, где пучок есть.
const NEAR_SHARE: f32 = 0.6;
/// Доля краёв травы у дорог и земли, где пучок есть.
const EDGE_SHARE: f32 = 0.12;
/// Доля клеток поля, где стоит кучка пучков.
const FIELD_SHARE: f32 = 0.012;

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

/// Трава ли в точке: покрытие «Трава» или «Сухая трава», не стена и не вода.
fn grass_at(map: &Map, p: Vec2) -> Option<u8> {
    let (x, y) = Map::cell_of(p);
    let i = map.index(x, y)?;
    let g = map.ground[i];
    (g <= 1 && map.solid[i] == 0 && map.water_depth(p) <= 0.02).then_some(g)
}

/// Пучки всей карты.
pub fn place(map: &Map, props: &[Prop], footprint: impl Fn(&str) -> Option<Footprint>) -> Vec<Tuft> {
    let mut out = Vec::new();
    let put = |p: Vec2, seed: (u32, u32, u32), big: bool, out: &mut Vec<Tuft>| {
        let Some(g) = grass_at(map, p) else { return };
        let h = |k: u32| hash(seed.0, seed.1, seed.2.wrapping_mul(16).wrapping_add(k));
        // Чуть светлее и желтее земли под ним — как на референсах RWR2.
        let base = if g == 1 { [0.74, 0.63, 0.39] } else { [0.60, 0.53, 0.28] };
        let v = 0.92 + 0.16 * h(1);
        let scale = if big { 1.1 + 0.6 * h(2) } else { 0.8 + 0.5 * h(2) };
        out.push(Tuft { pos: p, angle: h(3) * std::f32::consts::TAU, scale, tint: base.map(|c| c * v) });
    };

    // У предметов: по краю основания (дома, заборы, колодцы; у деревьев — ствол).
    for (n, prop) in props.iter().enumerate() {
        let Some([x0, z0, x1, z1]) = footprint(&prop.model) else { continue };
        let (sin, cos) = prop.angle.sin_cos();
        let s = prop.scale;
        let world = |x: f32, z: f32| prop.at() + Vec2::new(x * cos - z * sin, x * sin + z * cos) * s;
        let (w, d) = ((x1 - x0) * s, (z1 - z0) * s);
        let n = n as u32;
        if 2.0 * (w + d) < 3.0 {
            // Узкое основание (ствол, столб): два-три пучка вокруг.
            let cx = (x0 + x1) * 0.5;
            let cz = (z0 + z1) * 0.5;
            let count = 2 + (hash(n, 7, 1) * 1.99) as u32;
            for k in 0..count {
                let a = (k as f32 + hash(n, k, 2)) / count as f32 * std::f32::consts::TAU;
                let r = (w.max(d) * 0.5 + 0.15 + 0.3 * hash(n, k, 3)) / s;
                put(world(cx + a.cos() * r, cz + a.sin() * r), (n, k, 4), true, &mut out);
            }
            continue;
        }
        // Обходим прямоугольник основания с шагом STEP, отступая наружу.
        let sides = [((x0, z0), (x1, z0), (0.0, -1.0)), ((x1, z0), (x1, z1), (1.0, 0.0)), ((x1, z1), (x0, z1), (0.0, 1.0)), ((x0, z1), (x0, z0), (-1.0, 0.0))];
        for (side, ((ax, az), (bx, bz), (ox, oz))) in sides.into_iter().enumerate() {
            let len = ((bx - ax).abs() + (bz - az).abs()) * s;
            let steps = (len / STEP).ceil().max(1.0) as u32;
            for k in 0..steps {
                let seed = (n, side as u32 * 1000 + k, 5);
                if hash(seed.0, seed.1, seed.2) > NEAR_SHARE {
                    continue;
                }
                let t = (k as f32 + hash(seed.0, seed.1, 6)) / steps as f32;
                let out_m = (0.08 + 0.3 * hash(seed.0, seed.1, 7)) / s;
                let (x, z) = (ax + (bx - ax) * t + ox * out_m, az + (bz - az) * t + oz * out_m);
                put(world(x, z), seed, true, &mut out);
            }
        }
    }

    // По клеткам: у стен, по краю травы, кучки в поле.
    let dirs = [(1, 0), (-1, 0), (0, 1), (0, -1)];
    for y in 0..map.height as i32 {
        for x in 0..map.width as i32 {
            let i = y as usize * map.width as usize + x as usize;
            if map.ground[i] > 1 || map.solid[i] != 0 {
                continue;
            }
            let (ux, uy) = (x as u32, y as u32);
            let cell = Vec2::new(x as f32, y as f32);
            for (k, (dx, dy)) in dirs.into_iter().enumerate() {
                let Some(j) = map.index(x + dx, y + dy) else { continue };
                let wall = map.solid[j] != 0;
                let edge = !wall && map.ground[j] > 1;
                let share = if wall { NEAR_SHARE } else if edge { EDGE_SHARE } else { continue };
                let k = k as u32;
                if hash(ux, uy, 100 + k) > share {
                    continue;
                }
                // Точка у общей стороны клетки, отступ внутрь травы.
                let along = hash(ux, uy, 110 + k);
                let inset = if wall { 0.12 + 0.25 * hash(ux, uy, 120 + k) } else { 0.4 + 0.5 * hash(ux, uy, 120 + k) };
                let p = match (dx, dy) {
                    (1, 0) => cell + Vec2::new(1.0 - inset, along),
                    (-1, 0) => cell + Vec2::new(inset, along),
                    (0, 1) => cell + Vec2::new(along, 1.0 - inset),
                    _ => cell + Vec2::new(along, inset),
                };
                put(p, (ux, uy, 130 + k), wall, &mut out);
            }
            if hash(ux, uy, 200) < FIELD_SHARE {
                let count = 2 + (hash(ux, uy, 201) * 2.99) as u32;
                for k in 0..count {
                    let a = hash(ux, uy, 210 + k) * std::f32::consts::TAU;
                    let r = 0.6 * hash(ux, uy, 220 + k);
                    let p = cell + Vec2::splat(0.5) + Vec2::new(a.cos(), a.sin()) * r;
                    put(p, (ux, uy, 230 + k), false, &mut out);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Поле травы 40×40, дорога (земля) поперёк, стена из клеток и забор.
    fn sample() -> (Map, Vec<Prop>) {
        let mut map = Map::new(40, 40);
        for x in 0..40 {
            for y in 18..21 {
                map.ground[y * 40 + x] = 2;
            }
        }
        for x in 5..12 {
            map.set(x, 30, 255);
        }
        let fence = Prop { model: "забор".into(), pos: (25.0, 8.0), angle: 0.4, scale: 1.0 };
        (map, vec![fence])
    }

    fn footprint(name: &str) -> Option<Footprint> {
        (name == "забор").then_some([-2.0, -0.05, 2.0, 0.05])
    }

    #[test]
    fn tufts_stand_only_on_grass_and_gather_at_walls_and_fences() {
        let (map, props) = sample();
        let tufts = place(&map, &props, footprint);
        assert!(!tufts.is_empty());
        for t in &tufts {
            let (x, y) = Map::cell_of(t.pos);
            let i = map.index(x, y).expect("в пределах карты");
            assert!(map.ground[i] <= 1 && map.solid[i] == 0, "пучок не на траве: {:?}", t.pos);
        }
        let near = |c: Vec2, r: f32| tufts.iter().filter(|t| t.pos.distance(c) < r).count();
        // У забора длиной 4 м и у стены в 7 клеток — по несколько пучков, в поле — редко.
        assert!(near(Vec2::new(25.0, 8.0), 2.6) >= 3, "у забора мало пучков");
        assert!(tufts.iter().filter(|t| (t.pos.y - 30.0).abs() < 1.5 && (5.0..12.0).contains(&t.pos.x)).count() >= 4, "у стены мало пучков");
        let density = tufts.len() as f32 / (40.0 * 37.0);
        assert!(density < 0.15, "это уже ковёр: {density} пучка на м²");
    }

    #[test]
    fn placement_is_the_same_every_time() {
        let (map, props) = sample();
        assert_eq!(place(&map, &props, footprint), place(&map, &props, footprint));
    }
}
