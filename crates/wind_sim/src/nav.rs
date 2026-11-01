//! Поиск пути: граф чанков и поля расстояний внутри чанков.
//!
//! Солдату нужно знать, куда шагнуть, чтобы дойти до цели (точки, базы). Поле
//! расстояний по всей карте — дорого: на 4 км это 16 млн клеток на каждую цель. Поэтому
//! в два уровня:
//!
//! 1. **Граф чанков** ([`NavGraph`]). Карта поделена на чанки 32×32. На границе соседних
//!    чанков — проходы: куски до 16 клеток, где можно перейти. Узлы графа — середины
//!    проходов, рёбра — пути между ними внутри чанка. Граф общий для всех целей; после
//!    разрушений пересчитываются только задетые чанки.
//! 2. **Поле цели** ([`FlowField`]): расстояние от каждого узла до цели — Дейкстра по
//!    графу, узлов в сотни раз меньше, чем клеток. Поле по клеткам считается только
//!    в том чанке, куда заглянул солдат: края чанка «знают» расстояние до цели от
//!    проходов.
//!
//! Тысяча солдат, идущих к одной точке, стоят столько же, сколько один.

use crate::map::{EMPTY, Map};
use glam::Vec2;
use rayon::prelude::*;
use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::OnceLock;

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

/// Цена шага по прямой — и через границу чанка между серединами проходов.
const STEP: u32 = 5;
/// Самый длинный проход, клеток: длиннее — режем на куски, чтобы узел был недалеко от
/// каждой клетки прохода.
const SEGMENT: i32 = 16;

/// Расстояния по клеткам от `target` по всей карте (Дейкстра). Дорого на больших
/// картах — для проверок: всё ли достижимо (редактор, генератор, тесты).
pub fn distances(map: &Map, target: Vec2) -> Vec<u32> {
    let deep = map.deep_mask();
    let rect = Rect { x0: 0, y0: 0, w: map.width as i32, h: map.height as i32 };
    let mut dist = vec![UNREACHABLE; rect.len()];
    let (tx, ty) = Map::cell_of(target);
    if passable(map, &deep, tx, ty) {
        sweep(rect, &blockers(map, &deep, rect), &mut vec![(0, rect.local(tx, ty).unwrap() as u32)], &mut dist);
    }
    dist
}

/// Граф проходов между чанками — общий для всех целей.
pub struct NavGraph {
    /// Глубокая вода по клеткам (`Map::deep_mask`): туда не ходят. Высоты и вода в бою не
    /// меняются, поэтому маска считается раз.
    deep: Vec<bool>,
    chunks: Vec<ChunkNav>,
    chunks_x: usize,
    /// Номер первого узла чанка; узлы чанка — его проходы по порядку.
    first: Vec<u32>,
    /// Рёбра узлов подряд (`edge_start[n]..edge_start[n + 1]`): (куда, цена).
    edge_start: Vec<u32>,
    edges: Vec<(u32, u32)>,
    /// Версии чанков карты и её связности, по которым построен граф.
    chunk_revisions: Vec<u32>,
    topology_revision: u64,
    /// Растёт при каждой перестройке — поля целей сверяются с ней.
    pub version: u64,
}

/// Проходы чанка и пути между ними внутри него.
#[derive(Default)]
struct ChunkNav {
    /// Проходы по сторонам — запад, восток, север, юг: сторона `s` — это
    /// `pieces[sides[s]..sides[s + 1]]`.
    pieces: Vec<Piece>,
    sides: [u16; 5],
    /// Пути внутри чанка между серединами проходов: (откуда, куда, цена), откуда < куда.
    links: Vec<(u16, u16, u32)>,
}

/// Проход через сторону чанка: клетки `from`, `from + step`, … (`len` штук) внутри
/// чанка; из каждой можно шагнуть в соседний чанк.
#[derive(Clone, Copy, Debug)]
struct Piece {
    from: (i32, i32),
    step: (i32, i32),
    len: i32,
}

impl Piece {
    fn cell(&self, j: i32) -> (i32, i32) {
        (self.from.0 + self.step.0 * j, self.from.1 + self.step.1 * j)
    }

    fn mid(&self) -> i32 {
        self.len / 2
    }

    fn center(&self) -> (i32, i32) {
        self.cell(self.mid())
    }

    /// Где клетка `j` вдоль стороны чанка.
    fn along(&self, j: i32) -> i32 {
        let (x, y) = self.cell(j);
        if self.step.0 != 0 { x } else { y }
    }

    /// `next` начинается сразу за этим проходом — один сплошной проход, разрезанный на куски.
    fn joins(&self, next: &Piece) -> bool {
        self.along(self.len - 1) + 1 == next.along(0)
    }
}

/// Прямоугольник клеток; клетки внутри нумеруются построчно.
#[derive(Clone, Copy)]
struct Rect {
    x0: i32,
    y0: i32,
    w: i32,
    h: i32,
}

impl Rect {
    fn of_chunk(map: &Map, chunk: usize) -> Rect {
        let (x0, y0, w, h) = map.chunk_rect(chunk);
        Rect { x0: x0 as i32, y0: y0 as i32, w: w as i32, h: h as i32 }
    }

    fn len(&self) -> usize {
        (self.w * self.h) as usize
    }

    fn local(&self, x: i32, y: i32) -> Option<usize> {
        let (lx, ly) = (x - self.x0, y - self.y0);
        (lx >= 0 && ly >= 0 && lx < self.w && ly < self.h).then_some((ly * self.w + lx) as usize)
    }
}

/// Можно ли стоять в клетке: не стена, не глубокая вода, на карте.
fn passable(map: &Map, deep: &[bool], x: i32, y: i32) -> bool {
    map.index(x, y).is_some_and(|i| map.solid[i] == EMPTY && !deep.get(i).copied().unwrap_or(false))
}

/// По диагонали нельзя срезать угол стены (угол у глубокой воды — можно).
#[inline]
fn can_step(map: &Map, x: i32, y: i32, dx: i32, dy: i32) -> bool {
    !map.is_solid(x + dx, y + dy)
        && (dx == 0 || dy == 0 || (!map.is_solid(x + dx, y) && !map.is_solid(x, y + dy)))
}

/// Что в клетке прямоугольника мешает пройти: [`WALL`], [`DEEP`] или ничего.
fn blockers(map: &Map, deep: &[bool], rect: Rect) -> Vec<u8> {
    let mut out = Vec::with_capacity(rect.len());
    for y in rect.y0..rect.y0 + rect.h {
        let row = (y * map.width as i32 + rect.x0) as usize;
        for i in row..row + rect.w as usize {
            let wall = if map.solid[i] != EMPTY { WALL } else { 0 };
            let water = if deep.get(i).copied().unwrap_or(false) { DEEP } else { 0 };
            out.push(wall | water);
        }
    }
    out
}

/// Стена: в клетку не войти и угол её не срезать.
const WALL: u8 = 1;
/// Глубокая вода: в клетку не войти, угол срезать можно.
const DEEP: u8 = 2;

/// Дейкстра по клеткам внутри `rect` от нескольких начал со своими расстояниями
/// (`seeds`: расстояние, номер клетки в `rect`). `blocked` — из [`blockers`]. Очередь из
/// корзин (Dial): веса шагов маленькие и целые; начала вливаются в очередь, когда до
/// их расстояния дошли.
fn sweep(rect: Rect, blocked: &[u8], seeds: &mut Vec<(u32, u32)>, dist: &mut [u32]) {
    const BUCKETS: usize = 8; // больше самого дорогого шага
    dist.fill(UNREACHABLE);
    // С конца — самые близкие.
    seeds.sort_unstable_by_key(|s| Reverse(*s));
    let Some(&(mut current, _)) = seeds.last() else { return };
    let (w, h) = (rect.w, rect.h);
    let mut buckets: [Vec<u32>; BUCKETS] = Default::default();
    let mut pending = 0usize;
    loop {
        while let Some(&(d, i)) = seeds.last() {
            if d > current {
                break;
            }
            seeds.pop();
            if d < dist[i as usize] {
                dist[i as usize] = d;
                buckets[d as usize % BUCKETS].push(i);
                pending += 1;
            }
        }
        if pending == 0 {
            match seeds.last() {
                Some(&(d, _)) => {
                    current = d;
                    continue;
                }
                None => break,
            }
        }
        let b = current as usize % BUCKETS;
        while let Some(i) = buckets[b].pop() {
            pending -= 1;
            if dist[i as usize] != current {
                continue; // устаревшая запись: клетку уже нашли короче
            }
            let (x, y) = (i as i32 % w, i as i32 / w);
            for &(dx, dy, cost) in &NEIGHBORS {
                let (nx, ny) = (x + dx, y + dy);
                if nx < 0 || ny < 0 || nx >= w || ny >= h {
                    continue;
                }
                let j = (ny * w + nx) as usize;
                if blocked[j] != 0 {
                    continue;
                }
                // По диагонали нельзя срезать угол стены.
                if dx != 0 && dy != 0 && (blocked[(y * w + nx) as usize] & WALL != 0 || blocked[(ny * w + x) as usize] & WALL != 0) {
                    continue;
                }
                let nd = current + cost;
                if nd < dist[j] {
                    dist[j] = nd;
                    buckets[nd as usize % BUCKETS].push(j as u32);
                    pending += 1;
                }
            }
        }
        current += 1;
    }
}

/// Проходы через сторону `side` чанка (0 — запад, 1 — восток, 2 — север, 3 — юг).
fn scan_side(map: &Map, deep: &[bool], rect: Rect, side: usize) -> Vec<Piece> {
    let (w, h) = (map.width as i32, map.height as i32);
    let (from, step, len, out) = match side {
        0 => ((rect.x0, rect.y0), (0, 1), rect.h, (-1, 0)),
        1 => ((rect.x0 + rect.w - 1, rect.y0), (0, 1), rect.h, (1, 0)),
        2 => ((rect.x0, rect.y0), (1, 0), rect.w, (0, -1)),
        _ => ((rect.x0, rect.y0 + rect.h - 1), (1, 0), rect.w, (0, 1)),
    };
    let (ox, oy) = (from.0 + out.0, from.1 + out.1);
    if ox < 0 || oy < 0 || ox >= w || oy >= h {
        return Vec::new(); // край карты
    }
    let open = |j: i32| {
        let (x, y) = (from.0 + step.0 * j, from.1 + step.1 * j);
        passable(map, deep, x, y) && passable(map, deep, x + out.0, y + out.1)
    };
    let mut pieces = Vec::new();
    let mut j = 0;
    while j < len {
        if !open(j) {
            j += 1;
            continue;
        }
        let start = j;
        while j < len && open(j) {
            j += 1;
        }
        // Сплошной проход — на равные куски не длиннее SEGMENT.
        let run = j - start;
        let parts = (run + SEGMENT - 1) / SEGMENT;
        for k in 0..parts {
            let (a, b) = (start + run * k / parts, start + run * (k + 1) / parts);
            pieces.push(Piece { from: (from.0 + step.0 * a, from.1 + step.1 * a), step, len: b - a });
        }
    }
    pieces
}

/// Проходы чанка и пути между ними.
fn build_chunk(map: &Map, deep: &[bool], chunk: usize) -> ChunkNav {
    let rect = Rect::of_chunk(map, chunk);
    let mut nav = ChunkNav::default();
    for side in 0..4 {
        nav.pieces.extend(scan_side(map, deep, rect, side));
        nav.sides[side + 1] = nav.pieces.len() as u16;
    }
    let blocked = blockers(map, deep, rect);
    let mut dist = vec![UNREACHABLE; rect.len()];
    let mut seeds = Vec::new();
    for a in 0..nav.pieces.len().saturating_sub(1) {
        let (x, y) = nav.pieces[a].center();
        seeds.push((0, rect.local(x, y).unwrap() as u32));
        sweep(rect, &blocked, &mut seeds, &mut dist);
        for b in a + 1..nav.pieces.len() {
            let (x, y) = nav.pieces[b].center();
            let d = dist[rect.local(x, y).unwrap()];
            if d != UNREACHABLE {
                nav.links.push((a as u16, b as u16, d));
            }
        }
    }
    nav
}

impl NavGraph {
    pub fn build(map: &Map) -> NavGraph {
        let deep = map.deep_mask();
        let chunks = (0..map.chunk_revisions.len()).into_par_iter().map(|c| build_chunk(map, &deep, c)).collect();
        let mut nav = NavGraph {
            deep,
            chunks,
            chunks_x: map.chunks_x() as usize,
            first: Vec::new(),
            edge_start: Vec::new(),
            edges: Vec::new(),
            chunk_revisions: map.chunk_revisions.clone(),
            topology_revision: map.topology_revision,
            version: 0,
        };
        nav.link();
        nav
    }

    /// Карта стала связнее (пробили стену) — пересчитать задетые чанки. `false` — граф
    /// и так свежий.
    pub fn refresh(&mut self, map: &Map) -> bool {
        if map.topology_revision == self.topology_revision {
            return false;
        }
        // Проходы на границе зависят от обоих чанков — пересчитываем и соседей.
        let cx = self.chunks_x;
        let mut todo = vec![false; self.chunks.len()];
        for (c, (&now, &was)) in map.chunk_revisions.iter().zip(&self.chunk_revisions).enumerate() {
            if now == was {
                continue;
            }
            todo[c] = true;
            if c % cx > 0 {
                todo[c - 1] = true;
            }
            if c % cx + 1 < cx {
                todo[c + 1] = true;
            }
            if c >= cx {
                todo[c - cx] = true;
            }
            if c + cx < todo.len() {
                todo[c + cx] = true;
            }
        }
        let deep = &self.deep;
        let rebuilt: Vec<(usize, ChunkNav)> =
            todo.par_iter().enumerate().filter(|(_, t)| **t).map(|(c, _)| (c, build_chunk(map, deep, c))).collect();
        for (c, nav) in rebuilt {
            self.chunks[c] = nav;
        }
        self.chunk_revisions.clone_from(&map.chunk_revisions);
        self.topology_revision = map.topology_revision;
        self.link();
        true
    }

    pub fn node_count(&self) -> usize {
        self.edge_start.len().saturating_sub(1)
    }

    /// Чанк и номер прохода, соединённого с проходом `k` чанка `c` через границу.
    fn partner(&self, c: usize, k: usize) -> (usize, usize) {
        let nav = &self.chunks[c];
        let side = (0..4).find(|&s| k < nav.sides[s + 1] as usize).unwrap();
        let i = k - nav.sides[side] as usize;
        let (other, other_side) = match side {
            0 => (c - 1, 1),
            1 => (c + 1, 0),
            2 => (c - self.chunks_x, 3),
            _ => (c + self.chunks_x, 2),
        };
        (other, self.chunks[other].sides[other_side] as usize + i)
    }

    /// Собрать узлы и рёбра из проходов чанков.
    fn link(&mut self) {
        self.first.clear();
        let mut total = 0u32;
        for nav in &self.chunks {
            self.first.push(total);
            total += nav.pieces.len() as u32;
        }
        let mut degree = vec![0u32; total as usize];
        for (c, nav) in self.chunks.iter().enumerate() {
            let first = self.first[c] as usize;
            for d in &mut degree[first..first + nav.pieces.len()] {
                *d += 1; // через границу
            }
            for &(a, b, _) in &nav.links {
                degree[first + a as usize] += 1;
                degree[first + b as usize] += 1;
            }
        }
        self.edge_start.clear();
        self.edge_start.push(0);
        for d in &degree {
            let last = *self.edge_start.last().unwrap();
            self.edge_start.push(last + d);
        }
        let mut edges = vec![(0, 0); *self.edge_start.last().unwrap() as usize];
        let mut cursor: Vec<u32> = self.edge_start[..total as usize].to_vec();
        let mut put = |from: u32, to: u32, cost: u32| {
            edges[cursor[from as usize] as usize] = (to, cost);
            cursor[from as usize] += 1;
        };
        for c in 0..self.chunks.len() {
            let first = self.first[c];
            for k in 0..self.chunks[c].pieces.len() {
                let (oc, ok) = self.partner(c, k);
                put(first + k as u32, self.first[oc] + ok as u32, STEP);
            }
            for &(a, b, cost) in &self.chunks[c].links {
                put(first + a as u32, first + b as u32, cost);
                put(first + b as u32, first + a as u32, cost);
            }
        }
        self.edges = edges;
        self.version += 1;
    }

    fn edges_of(&self, n: u32) -> &[(u32, u32)] {
        &self.edges[self.edge_start[n as usize] as usize..self.edge_start[n as usize + 1] as usize]
    }
}

/// Куда идти к одной цели: расстояния от узлов графа и поля по клеткам в чанках.
pub struct FlowField {
    pub target: Vec2,
    /// Расстояние от каждого узла графа до цели.
    nodes: Vec<u32>,
    /// Поле по клеткам каждого чанка — считается, когда в чанк впервые заглянут.
    cells: Vec<OnceLock<Box<[u32]>>>,
    /// Версия графа, по которой посчитано.
    version: u64,
}

impl FlowField {
    pub fn new(target: Vec2) -> Self {
        FlowField { target, nodes: Vec::new(), cells: Vec::new(), version: u64::MAX }
    }

    pub fn is_stale(&self, nav: &NavGraph) -> bool {
        self.version != nav.version
    }

    /// Расстояния от узлов до цели — Дейкстра по графу. Поля в чанках сбрасываются.
    pub fn compute(&mut self, nav: &NavGraph, map: &Map) {
        self.version = nav.version;
        self.cells = (0..nav.chunks.len()).map(|_| OnceLock::new()).collect();
        self.nodes.clear();
        self.nodes.resize(nav.node_count(), UNREACHABLE);
        let (tx, ty) = Map::cell_of(self.target);
        if !passable(map, &nav.deep, tx, ty) {
            return;
        }
        // До проходов своего чанка — по клеткам, дальше — по графу.
        let chunk = map.chunk_of_cell(tx, ty);
        let rect = Rect::of_chunk(map, chunk);
        let mut dist = vec![UNREACHABLE; rect.len()];
        sweep(rect, &blockers(map, &nav.deep, rect), &mut vec![(0, rect.local(tx, ty).unwrap() as u32)], &mut dist);
        let mut heap = BinaryHeap::new();
        for (k, p) in nav.chunks[chunk].pieces.iter().enumerate() {
            let (x, y) = p.center();
            let d = dist[rect.local(x, y).unwrap()];
            if d != UNREACHABLE {
                let n = nav.first[chunk] + k as u32;
                self.nodes[n as usize] = d;
                heap.push(Reverse((d, n)));
            }
        }
        while let Some(Reverse((d, n))) = heap.pop() {
            if d != self.nodes[n as usize] {
                continue;
            }
            for &(m, cost) in nav.edges_of(n) {
                let nd = d + cost;
                if nd < self.nodes[m as usize] {
                    self.nodes[m as usize] = nd;
                    heap.push(Reverse((nd, m)));
                }
            }
        }
    }

    /// Расстояние от клетки до цели (шаг — 5). Поле чанка считается при первом обращении.
    pub fn distance(&self, nav: &NavGraph, map: &Map, x: i32, y: i32) -> u32 {
        if map.index(x, y).is_none() {
            return UNREACHABLE;
        }
        let chunk = map.chunk_of_cell(x, y);
        let Some(cell) = self.cells.get(chunk) else { return UNREACHABLE };
        let field = cell.get_or_init(|| self.chunk_field(nav, map, chunk));
        field[Rect::of_chunk(map, chunk).local(x, y).unwrap()]
    }

    /// Поле по клеткам чанка: с краёв — расстояния от проходов, внутри — Дейкстра.
    fn chunk_field(&self, nav: &NavGraph, map: &Map, chunk: usize) -> Box<[u32]> {
        let rect = Rect::of_chunk(map, chunk);
        let c = &nav.chunks[chunk];
        let first = nav.first[chunk] as usize;
        let mut seeds = Vec::new();
        for s in 0..4 {
            let (a, b) = (c.sides[s] as usize, c.sides[s + 1] as usize);
            let side = &c.pieces[a..b];
            let node = |k: usize| self.nodes[first + a + k];
            for (k, p) in side.iter().enumerate() {
                let d = node(k);
                if d == UNREACHABLE {
                    continue;
                }
                // Соседние куски того же прохода: между их серединами расстояние меняется
                // плавно — иначе в чистом поле солдат шёл бы к проходу ровно поперёк границы,
                // лесенкой.
                let prev = k.checked_sub(1).filter(|&q| side[q].joins(p)).map(|q| (side[q], node(q)));
                let next = side.get(k + 1).filter(|q| p.joins(q)).map(|q| (*q, node(k + 1)));
                for j in 0..p.len {
                    let (x, y) = p.cell(j);
                    seeds.push((seed(p, d, prev, next, j), rect.local(x, y).unwrap() as u32));
                }
            }
        }
        let (tx, ty) = Map::cell_of(self.target);
        if let Some(i) = rect.local(tx, ty)
            && passable(map, &nav.deep, tx, ty)
        {
            seeds.push((0, i as u32));
        }
        let mut dist = vec![UNREACHABLE; rect.len()];
        sweep(rect, &blockers(map, &nav.deep, rect), &mut seeds, &mut dist);
        dist.into_boxed_slice()
    }

    /// Куда идти из `pos`. `None`, если отсюда до цели не дойти.
    pub fn direction(&self, nav: &NavGraph, map: &Map, pos: Vec2) -> Option<Vec2> {
        let (x, y) = Map::cell_of(pos);
        let here = self.distance(nav, map, x, y);
        if here == UNREACHABLE {
            return None;
        }
        if here == 0 {
            return Some((self.target - pos).normalize_or_zero());
        }
        let mut best = here;
        let mut best_cell = None;
        // Глубокие клетки недостижимы — их не выберем; а срезать угол у воды можно.
        for &(dx, dy, _) in &NEIGHBORS {
            if !can_step(map, x, y, dx, dy) {
                continue;
            }
            let d = self.distance(nav, map, x + dx, y + dy);
            if d < best {
                best = d;
                best_cell = Some((x + dx, y + dy));
            }
        }
        let (bx, by) = best_cell?;
        Some((Vec2::new(bx as f32 + 0.5, by as f32 + 0.5) - pos).normalize_or_zero())
    }
}

/// Расстояние до цели в клетке `j` прохода `p`, у середины которого оно `d`.
///
/// Наверняка не больше, чем дойти вдоль прохода до середины своего или соседнего куска
/// (шаг — 5 на клетку). Между серединами соседних кусков — ровно по прямой от одной
/// к другой, если она заметно убывает: так в чистом поле солдат идёт к цели наискось,
/// а не лесенкой. Ямок, из которых некуда шагнуть, так не бывает: значение вдоль прохода
/// убывает к какой-нибудь середине, а от неё по графу — к цели.
fn seed(p: &Piece, d: u32, prev: Option<(Piece, u32)>, next: Option<(Piece, u32)>, j: i32) -> u32 {
    let at = p.along(j);
    let center = p.along(p.mid());
    let cone = |c: i32, dc: u32| dc as f32 + STEP as f32 * (at - c).abs() as f32;
    let mut v = cone(center, d);
    for (q, dq) in [prev, next].into_iter().flatten().filter(|q| q.1 != UNREACHABLE) {
        let other = q.along(q.mid());
        v = v.min(cone(other, dq));
        let (t, span) = (at - center, other - center);
        if t * span > 0 && t.abs() < span.abs() {
            let slope = (dq as f32 - d as f32) / span as f32;
            if slope.abs() >= 1.0 {
                v = v.min(d as f32 + slope * t as f32);
            }
        }
    }
    v.round() as u32
}
