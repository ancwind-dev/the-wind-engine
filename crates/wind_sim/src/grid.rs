//! Пространственная сетка: быстро отвечает «кто находится рядом с точкой».
//!
//! Без неё каждый солдат проверял бы каждого (1000 × 1000 = миллион проверок за тик).
//! Пересобирается целиком каждый тик сортировкой подсчётом — это дешевле, чем
//! поддерживать её по частям.

use glam::Vec2;

pub struct SpatialGrid {
    cell_size: f32,
    cols: i32,
    rows: i32,
    /// Объекты клетки `c` лежат в `items[starts[c]..starts[c + 1]]`.
    starts: Vec<u32>,
    items: Vec<u32>,
    item_cells: Vec<u32>,
    cursor: Vec<u32>,
}

const NO_CELL: u32 = u32::MAX;

impl SpatialGrid {
    pub fn new(world_size: f32, cell_size: f32) -> Self {
        let cols = (world_size / cell_size).ceil().max(1.0) as i32;
        let cells = (cols * cols) as usize;
        SpatialGrid {
            cell_size,
            cols,
            rows: cols,
            starts: vec![0; cells + 1],
            items: Vec::new(),
            item_cells: Vec::new(),
            cursor: vec![0; cells],
        }
    }

    fn cell_coords(&self, pos: Vec2) -> (i32, i32) {
        let x = ((pos.x / self.cell_size) as i32).clamp(0, self.cols - 1);
        let y = ((pos.y / self.cell_size) as i32).clamp(0, self.rows - 1);
        (x, y)
    }

    /// `pos_of(i)` возвращает позицию объекта `i` или `None`, если его не нужно
    /// класть в сетку (например, солдат мёртв).
    pub fn rebuild(&mut self, len: usize, pos_of: impl Fn(usize) -> Option<Vec2>) {
        self.starts.fill(0);
        self.item_cells.clear();
        for i in 0..len {
            let cell = match pos_of(i) {
                Some(pos) => {
                    let (x, y) = self.cell_coords(pos);
                    let c = (y * self.cols + x) as usize;
                    self.starts[c + 1] += 1;
                    c as u32
                }
                None => NO_CELL,
            };
            self.item_cells.push(cell);
        }
        for c in 1..self.starts.len() {
            self.starts[c] += self.starts[c - 1];
        }
        let total = *self.starts.last().unwrap() as usize;
        self.items.resize(total, 0);
        self.cursor.copy_from_slice(&self.starts[..self.starts.len() - 1]);
        for (i, &cell) in self.item_cells.iter().enumerate() {
            if cell != NO_CELL {
                let slot = &mut self.cursor[cell as usize];
                self.items[*slot as usize] = i as u32;
                *slot += 1;
            }
        }
    }

    /// Вызывает `f` для каждого объекта из клеток, задетых кругом.
    /// Объекты могут оказаться чуть дальше радиуса — точное расстояние проверяет вызывающий.
    pub fn query(&self, center: Vec2, radius: f32, mut f: impl FnMut(u32)) {
        let (x0, y0) = self.cell_coords(center - Vec2::splat(radius));
        let (x1, y1) = self.cell_coords(center + Vec2::splat(radius));
        for y in y0..=y1 {
            for x in x0..=x1 {
                let c = (y * self.cols + x) as usize;
                for &item in &self.items[self.starts[c] as usize..self.starts[c + 1] as usize] {
                    f(item);
                }
            }
        }
    }
}
