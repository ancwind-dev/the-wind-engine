//! Пространственная сетка: быстро отвечает «кто находится рядом с точкой».
//!
//! Без неё каждый солдат проверял бы каждого (1000 × 1000 = миллион проверок за тик).
//! Пересобирается целиком каждый тик сортировкой пар «клетка, объект» — цена зависит от
//! числа объектов, а не от размера карты (на 4 км клеток сетки — миллион).

use glam::Vec2;

pub struct SpatialGrid {
    cell_size: f32,
    cols: i32,
    rows: i32,
    /// Объекты по клеткам: клетка в старших 32 битах, номер объекта — в младших;
    /// по возрастанию — клетки построчно, в клетке — по номеру.
    sorted: Vec<u64>,
}

impl SpatialGrid {
    pub fn new(world_size: f32, cell_size: f32) -> Self {
        let cols = (world_size / cell_size).ceil().max(1.0) as i32;
        SpatialGrid { cell_size, cols, rows: cols, sorted: Vec::new() }
    }

    fn cell_coords(&self, pos: Vec2) -> (i32, i32) {
        let x = ((pos.x / self.cell_size) as i32).clamp(0, self.cols - 1);
        let y = ((pos.y / self.cell_size) as i32).clamp(0, self.rows - 1);
        (x, y)
    }

    fn key(&self, x: i32, y: i32) -> u64 {
        ((y * self.cols + x) as u64) << 32
    }

    /// `pos_of(i)` возвращает позицию объекта `i` или `None`, если его не нужно
    /// класть в сетку (например, солдат мёртв).
    pub fn rebuild(&mut self, len: usize, pos_of: impl Fn(usize) -> Option<Vec2>) {
        self.sorted.clear();
        for i in 0..len {
            if let Some(pos) = pos_of(i) {
                let (x, y) = self.cell_coords(pos);
                self.sorted.push(self.key(x, y) | i as u64);
            }
        }
        self.sorted.sort_unstable();
    }

    /// Вызывает `f` для каждого объекта из клеток, задетых кругом.
    /// Объекты могут оказаться чуть дальше радиуса — точное расстояние проверяет вызывающий.
    pub fn query(&self, center: Vec2, radius: f32, mut f: impl FnMut(u32)) {
        let (x0, y0) = self.cell_coords(center - Vec2::splat(radius));
        let (x1, y1) = self.cell_coords(center + Vec2::splat(radius));
        for y in y0..=y1 {
            // Клетки строки идут подряд: от первой до последней — один кусок списка.
            let (lo, hi) = (self.key(x0, y), self.key(x1, y) + (1 << 32));
            let from = self.sorted.partition_point(|&k| k < lo);
            for &k in self.sorted[from..].iter().take_while(|&&k| k < hi) {
                f(k as u32);
            }
        }
    }
}
