//! Поиск пути через поле расстояний (flow field).
//!
//! Вместо того чтобы искать путь для каждого солдата, считаем один раз для точки
//! назначения расстояние от каждой клетки карты до неё. Солдату остаётся шагнуть
//! в соседнюю клетку с меньшим расстоянием. Тысяча солдат, идущих к одной точке,
//! стоят столько же, сколько один.

use crate::map::Map;
use glam::Vec2;

pub const UNREACHABLE: u32 = u32::MAX;

/// Шаг по прямой стоит 5, по диагонали 7 (≈ 5·√2).
const NEIGHBORS: [(i32, i32, u32); 8] = [
    (1, 0, 5),
    (-1, 0, 5),
    (0, 1, 5),
    (0, -1, 5),
    (1, 1, 7),
    (1, -1, 7),
    (-1, 1, 7),
    (-1, -1, 7),
];

pub struct FlowField {
    pub target: Vec2,
    pub dist: Vec<u32>,
    /// Для какой версии карты посчитано поле.
    topology_revision: u64,
}

impl FlowField {
    pub fn new(target: Vec2) -> Self {
        FlowField { target, dist: Vec::new(), topology_revision: u64::MAX }
    }

    pub fn is_stale(&self, map: &Map) -> bool {
        self.topology_revision != map.topology_revision
    }

    /// Алгоритм Дейкстры с очередью из корзин (Dial): веса маленькие и целые,
    /// поэтому куча не нужна.
    pub fn compute(&mut self, map: &Map) {
        self.topology_revision = map.topology_revision;
        self.dist.clear();
        self.dist.resize((map.width * map.height) as usize, UNREACHABLE);
        let (tx, ty) = Map::cell_of(self.target);
        let Some(start) = map.index(tx, ty) else { return };
        if map.is_solid(tx, ty) {
            return;
        }

        const BUCKETS: usize = 8; // больше максимального веса шага
        let mut buckets: [Vec<u32>; BUCKETS] = Default::default();
        self.dist[start] = 0;
        buckets[0].push(start as u32);
        let mut pending = 1usize;
        let mut current = 0u32;
        let width = map.width as i32;

        while pending > 0 {
            let b = current as usize % BUCKETS;
            while let Some(i) = buckets[b].pop() {
                pending -= 1;
                if self.dist[i as usize] != current {
                    continue; // устаревшая запись: клетку уже нашли короче
                }
                let (x, y) = (i as i32 % width, i as i32 / width);
                for &(dx, dy, cost) in &NEIGHBORS {
                    if !can_step(map, x, y, dx, dy) {
                        continue;
                    }
                    let j = ((y + dy) * width + x + dx) as usize;
                    let nd = current + cost;
                    if nd < self.dist[j] {
                        self.dist[j] = nd;
                        buckets[nd as usize % BUCKETS].push(j as u32);
                        pending += 1;
                    }
                }
            }
            current += 1;
        }
    }

    /// Куда идти из `pos`. `None`, если отсюда до цели не дойти.
    pub fn direction(&self, map: &Map, pos: Vec2) -> Option<Vec2> {
        let (x, y) = Map::cell_of(pos);
        let here = self.dist[map.index(x, y)?];
        if here == 0 {
            return Some((self.target - pos).normalize_or_zero());
        }
        let mut best = here;
        let mut best_cell = None;
        for &(dx, dy, _) in &NEIGHBORS {
            if !can_step(map, x, y, dx, dy) {
                continue;
            }
            let d = self.dist[map.index(x + dx, y + dy)?];
            if d < best {
                best = d;
                best_cell = Some((x + dx, y + dy));
            }
        }
        let (bx, by) = best_cell?;
        Some((Vec2::new(bx as f32 + 0.5, by as f32 + 0.5) - pos).normalize_or_zero())
    }
}

/// По диагонали нельзя срезать угол стены.
#[inline]
fn can_step(map: &Map, x: i32, y: i32, dx: i32, dy: i32) -> bool {
    !map.is_solid(x + dx, y + dy)
        && (dx == 0 || dy == 0 || (!map.is_solid(x + dx, y) && !map.is_solid(x, y + dy)))
}
