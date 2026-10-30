//! Тела убитых — рэгдоллы, как в RWR: готовых поз нет, скелет модели (точки и палки)
//! падает сам — от толчка пули или взрыва и под силой тяжести — и ложится как придётся.
//!
//! Физика — Верле: у точки текущее и прошлое положение, скорость — их разница. Палки
//! держат длину; распорки между остальными точками не дают телу сложиться в комок,
//! а корпусу — провернуться. Лёгкие точки (руки) двигаются сильнее тяжёлых (корпус):
//! массы берём из файла модели (`invMass`). Земля — по высоте карты, стены — по клеткам.
//!
//! Это только картинка: у каждого клиента тело падает по-своему, по сети уходит лишь
//! толчок при смерти (`Soldier::death_push`).

use crate::mapmesh::wall_height;
use crate::rwr::VOXEL;
use glam::{Vec2, Vec3};
use wind_sim::map::Map;
use wind_sim::rng::Rng;

/// Шаг физики, с: мельче кадра, чтобы палки не растягивались на быстром падении.
const STEP: f32 = 1.0 / 120.0;
/// Сила тяжести, м/с². Чуть больше настоящей — тело падает резче, как в RWR.
const GRAVITY: f32 = 12.0;
/// Сколько раз за шаг поправляем палки и распорки.
const ITERATIONS: usize = 6;
/// Какая доля скорости остаётся за шаг в воздухе.
const DRAG: f32 = 0.998;
/// Какая доля скорости вдоль земли остаётся за шаг у точки, лежащей на земле.
const GROUND_KEEP: f32 = 0.86;
/// Толщина тела вокруг точек скелета, м: на столько точки держатся над землёй.
const RADIUS: f32 = 0.09;
/// Тело улеглось, если `REST_STEPS` шагов подряд все точки медленнее этого, м/с.
const REST_SPEED: f32 = 0.12;
const REST_STEPS: u32 = 48;
/// Дольше тело не считаем — замирает как есть.
const MAX_TIME: f32 = 6.0;

/// Связь двух точек: расстояние держится в пределах `min..=max`, м.
#[derive(Clone, Copy, Debug)]
struct Link {
    a: usize,
    b: usize,
    min: f32,
    max: f32,
}

/// Как устроено тело модели для физики. Строится один раз при загрузке модели.
#[derive(Clone, Debug, Default)]
pub struct Rig {
    links: Vec<Link>,
    /// Обратная масса точки: чем больше, тем легче её сдвинуть.
    inv_mass: Vec<f32>,
    /// Высота точки в позе покоя: 0 — стопы, 1 — макушка. Пуля толкает верх сильнее.
    height: Vec<f32>,
    /// Точка посередине корпуса — под ней лужа крови.
    center: usize,
}

impl Rig {
    /// `torso` — точки корпуса (шея, поясница, плечи, бёдра), `head` — голова;
    /// без них тело держат только палки и общие распорки.
    pub(crate) fn new(rest: &[Vec3], sticks: &[(usize, usize)], inv_mass: &[f32], torso: Option<([usize; 6], usize)>) -> Rig {
        let n = rest.len();
        let dist = |a: usize, b: usize| rest[a].distance(rest[b]) * VOXEL;
        let mut links: Vec<Link> = sticks.iter().map(|&(a, b)| Link { a, b, min: dist(a, b), max: dist(a, b) }).collect();
        let linked = |a: usize, b: usize| sticks.iter().any(|&(x, y)| (x, y) == (a, b) || (y, x) == (a, b));
        let in_torso = |i: usize| torso.is_some_and(|(t, _)| t.contains(&i));
        for a in 0..n {
            for b in a + 1..n {
                if linked(a, b) {
                    continue;
                }
                let d = dist(a, b);
                let (min, max) = if in_torso(a) && in_torso(b) {
                    // Корпус почти жёсткий: чуть гнётся, но не скручивается.
                    (d * 0.9, d * 1.05)
                } else if torso.is_some_and(|(t, head)| (a == head && t.contains(&b)) || (b == head && t.contains(&a))) {
                    // Голова кивает, но не уходит в грудь.
                    (d * 0.85, d * 1.02)
                } else {
                    // Руки и ноги гнутся, но не складываются пополам и не проходят
                    // сквозь тело.
                    (d * 0.4, f32::INFINITY)
                };
                links.push(Link { a, b, min, max });
            }
        }
        let top = rest.iter().map(|p| p.y).fold(f32::MIN, f32::max).max(1.0);
        let bottom = rest.iter().map(|p| p.y).fold(f32::MAX, f32::min).min(top - 1.0);
        let inv_mass = (0..n).map(|i| inv_mass.get(i).copied().filter(|m| *m > 0.0).unwrap_or(1.0)).collect();
        let height = rest.iter().map(|p| ((p.y - bottom) / (top - bottom)).clamp(0.0, 1.0)).collect();
        // Середина корпуса — точка, ближе всех к центру тела.
        let mid = rest.iter().copied().sum::<Vec3>() / n.max(1) as f32;
        let center = (0..n).min_by(|&a, &b| rest[a].distance(mid).total_cmp(&rest[b].distance(mid))).unwrap_or(0);
        Rig { links, inv_mass, height, center }
    }
}

/// Тело убитого.
#[derive(Clone, Debug)]
pub struct Ragdoll {
    /// Какая модель (`CharacterSet::models`).
    pub model: usize,
    /// Возраст тела, который пришёл с последним кадром (см. `SceneBuilder`).
    pub age: f32,
    pos: Vec<Vec3>,
    prev: Vec<Vec3>,
    ground: Vec<f32>,
    /// Сколько секунд тело уже падает.
    time: f32,
    still: u32,
    resting: bool,
    pending: f32,
}

impl Ragdoll {
    /// `points` — где точки скелета в момент смерти (мир, м). `push` — толчок, м/с
    /// по земле; `blast` — от взрыва: тело подбрасывает. `seed` — чтобы тела падали
    /// по-разному.
    pub fn new(model: usize, rig: &Rig, points: Vec<Vec3>, push: Vec2, blast: bool, seed: u32) -> Ragdoll {
        let mut rng = Rng::for_entity(0x07A6_D011, seed as u64, seed);
        let speed = push.length();
        let along = Vec3::new(push.x, 0.0, push.y).normalize_or_zero();
        let side = Vec3::new(-along.z, 0.0, along.x);
        // Немного вбок и с поворотом — чтобы тела не падали одинаково.
        let twist = rng.range(-0.45, 0.45);
        let lift = if blast { speed * rng.range(0.4, 0.6) } else { 0.0 };
        let mut jitter = |k: f32| Vec3::new(rng.range(-k, k), rng.range(-k, k) * 0.5, rng.range(-k, k));
        let prev = points
            .iter()
            .enumerate()
            .map(|(i, &p)| {
                let h = rig.height.get(i).copied().unwrap_or(0.5);
                let v = if blast {
                    // Взрыв уносит всё тело, верх — чуть сильнее.
                    along * speed * (0.75 + 0.35 * h) + Vec3::Y * lift * (0.7 + 0.6 * h) + jitter(0.9)
                } else {
                    // Пуля бьёт в корпус: верх уходит назад, ноги остаются — тело
                    // опрокидывается.
                    let w = 0.1 + 0.9 * h * h;
                    (along + side * twist) * speed * w + jitter(0.35)
                };
                p - v * STEP
            })
            .collect();
        let n = points.len();
        Ragdoll { model, age: 0.0, pos: points, prev, ground: vec![0.0; n], time: 0.0, still: 0, resting: false, pending: 0.0 }
    }

    /// Точки скелета, мир, м.
    pub fn points(&self) -> &[Vec3] {
        &self.pos
    }

    /// Сколько секунд тело уже падает.
    pub fn time(&self) -> f32 {
        self.time
    }

    /// Улеглось — больше не считается.
    pub fn resting(&self) -> bool {
        self.resting
    }

    /// Середина корпуса на земле — для лужи крови.
    pub fn center(&self, rig: &Rig) -> Vec2 {
        let p = self.pos.get(rig.center).copied().unwrap_or_default();
        Vec2::new(p.x, p.z)
    }

    /// Поза для `CharModel::bones`: точки в вокселях относительно `origin`.
    pub fn pose(&self, origin: Vec3) -> Vec<Vec3> {
        self.pos.iter().map(|&p| (p - origin) / VOXEL).collect()
    }

    /// Прошло `dt` секунд.
    pub fn advance(&mut self, rig: &Rig, map: &Map, dt: f32) {
        if self.resting {
            return;
        }
        self.pending += dt.max(0.0);
        while self.pending >= STEP && !self.resting {
            self.pending -= STEP;
            self.step(rig, map);
        }
    }

    fn step(&mut self, rig: &Rig, map: &Map) {
        let n = self.pos.len().min(rig.inv_mass.len());
        let fall = Vec3::NEG_Y * GRAVITY * STEP * STEP;
        for i in 0..n {
            let v = (self.pos[i] - self.prev[i]) * DRAG;
            self.prev[i] = self.pos[i];
            self.pos[i] += v + fall;
            self.ground[i] = map.height_at(Vec2::new(self.pos[i].x, self.pos[i].z)) + RADIUS;
        }
        for _ in 0..ITERATIONS {
            for l in &rig.links {
                let d = self.pos[l.b] - self.pos[l.a];
                let len = d.length();
                let goal = len.clamp(l.min, l.max);
                if len < 1e-6 || goal == len {
                    continue;
                }
                let (wa, wb) = (rig.inv_mass[l.a], rig.inv_mass[l.b]);
                let fix = d * ((len - goal) / len / (wa + wb));
                self.pos[l.a] += fix * wa;
                self.pos[l.b] -= fix * wb;
            }
            for i in 0..n {
                self.pos[i].y = self.pos[i].y.max(self.ground[i]);
            }
        }
        let mut fastest = 0.0f32;
        for i in 0..n {
            let (p, q) = (self.pos[i], self.prev[i]);
            // Стена: в клетку стены ниже её верха точка не заходит — остаётся, где была.
            let (x, y) = Map::cell_of(Vec2::new(p.x, p.z));
            if let Some(cell) = map.index(x, y)
                && map.solid[cell] != 0
                && p.y < self.ground[i] + wall_height(map.solid[cell])
                && !map.is_solid_at(Vec2::new(q.x, q.z))
            {
                self.pos[i].x = q.x;
                self.pos[i].z = q.z;
            }
            // На земле — трение, и вниз больше не падает.
            if self.pos[i].y <= self.ground[i] + 0.002 {
                let mut v = self.pos[i] - self.prev[i];
                v.x *= GROUND_KEEP;
                v.z *= GROUND_KEEP;
                v.y = v.y.max(0.0);
                self.prev[i] = self.pos[i] - v;
            }
            fastest = fastest.max((self.pos[i] - self.prev[i]).length());
        }
        self.time += STEP;
        self.still = if fastest < REST_SPEED * STEP { self.still + 1 } else { 0 };
        self.resting = self.still >= REST_STEPS || self.time >= MAX_TIME;
    }
}

/// Где точки скелета живого солдата в мире, м: поза `pose` (воксели, +X вперёд),
/// солдат стоит в `at` и смотрит под углом `facing`.
pub fn world_points(pose: &[Vec3], at: Vec3, facing: f32) -> Vec<Vec3> {
    let rot = glam::Mat3::from_rotation_y(-facing);
    pose.iter().map(|&p| at + rot * (p * VOXEL)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::character::{CharModel, CharacterSet, Motion};
    use std::path::Path;

    fn set() -> CharacterSet {
        CharacterSet::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data")).unwrap()
    }

    /// Убитый пулей с запада (толчок на восток) посреди ровной карты; ждём, пока уляжется.
    fn fall(m: &CharModel, map: &Map, at: Vec2, push: Vec2, blast: bool, seed: u32) -> Ragdoll {
        let pose = m.pose(Motion::default());
        let mut r = Ragdoll::new(0, &m.rig, world_points(&pose, Vec3::new(at.x, 0.0, at.y), 0.0), push, blast, seed);
        for _ in 0..600 {
            r.advance(&m.rig, map, 1.0 / 60.0);
        }
        r
    }

    fn lengths_kept(m: &CharModel, r: &Ragdoll) -> f32 {
        m.sticks
            .iter()
            .map(|&(a, b)| {
                let rest = m.rest[a].distance(m.rest[b]) * VOXEL;
                (r.points()[a].distance(r.points()[b]) - rest).abs() / rest.max(1e-3)
            })
            .fold(0.0, f32::max)
    }

    #[test]
    fn shot_soldier_falls_back_and_lies_still() {
        let set = set();
        let map = Map::new(64, 64);
        let at = Vec2::splat(32.0);
        for m in &set.models {
            let r = fall(m, &map, at, Vec2::new(wind_sim::world::BULLET_PUSH, 0.0), false, 1);
            assert!(r.resting(), "{}: тело не улеглось за 10 с", m.name);
            let top = r.points().iter().map(|p| p.y).fold(f32::MIN, f32::max);
            assert!(top < 0.45, "{}: тело не легло, верх на {top:.2} м", m.name);
            assert!(r.points().iter().all(|p| p.y >= RADIUS - 0.01), "{}: ушло под землю", m.name);
            // Отпало назад — по ходу пули, а не вперёд.
            let c = r.center(&m.rig);
            assert!(c.x - at.x > 0.2, "{}: корпус не отпал по ходу пули ({:.2} м)", m.name, c.x - at.x);
            let stretch = lengths_kept(m, &r);
            assert!(stretch < 0.05, "{}: палки растянулись на {:.0} %", m.name, stretch * 100.0);
        }
    }

    #[test]
    fn blast_throws_farther_and_poses_differ() {
        let set = set();
        let m = &set.models[0];
        let map = Map::new(64, 64);
        let at = Vec2::splat(32.0);
        let shot = fall(m, &map, at, Vec2::new(wind_sim::world::BULLET_PUSH, 0.0), false, 1).center(&m.rig).distance(at);
        let blast = fall(m, &map, at, Vec2::new(wind_sim::world::BLAST_PUSH, 0.0), true, 1).center(&m.rig).distance(at);
        println!("отлетело: от пули {shot:.2} м, от взрыва {blast:.2} м");
        assert!(blast > shot + 0.3, "взрыв должен отбрасывать дальше пули: {blast:.2} против {shot:.2}");
        // Разные тела — разные позы.
        let a = fall(m, &map, at, Vec2::new(wind_sim::world::BULLET_PUSH, 0.0), false, 1);
        let b = fall(m, &map, at, Vec2::new(wind_sim::world::BULLET_PUSH, 0.0), false, 2);
        let diff = a.points().iter().zip(b.points()).map(|(p, q)| p.distance(*q)).fold(0.0, f32::max);
        assert!(diff > 0.1, "два тела легли одинаково ({diff:.3} м)");
    }

    #[test]
    fn body_does_not_pass_through_walls() {
        let set = set();
        let m = &set.models[0];
        let mut map = Map::new(64, 64);
        // Стена в метре к востоку, взрыв толкает туда.
        for y in 20..44 {
            map.set(33, y, 200);
        }
        let r = fall(m, &map, Vec2::new(32.0, 32.0), Vec2::new(wind_sim::world::BLAST_PUSH, 0.0), true, 3);
        let far = r.points().iter().map(|p| p.x).fold(f32::MIN, f32::max);
        assert!(far < 33.0, "тело прошло сквозь стену (x = {far:.2})");
    }
}
