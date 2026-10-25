//! Редактор карт: документ, инструменты, отмена. Без интерфейса — его рисует `editor_ui`.
//!
//! Любое изменение идёт «мазком»: от нажатия до отпускания кнопки мыши. Перед первым
//! изменением чанка его содержимое запоминается — так мазок можно отменить и вернуть.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use wind_sim::map::{CHUNK, EMPTY, INDESTRUCTIBLE, Map};
use wind_sim::mapfile::MapFile;
use wind_sim::mapgen::Layout;
use wind_sim::nav::{FlowField, UNREACHABLE};
use wind_sim::Vec2;

pub const HOUSE_WALL: u8 = 120;
pub const FENCE: u8 = 40;
/// Больше точек захвата нельзя: они подписаны буквами A–Z.
pub const MAX_POINTS: usize = 26;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tool {
    Walls,
    Terrain,
    Ground,
    Points,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum WallShape {
    Brush,
    Line,
    Rect,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TerrainMode {
    Raise,
    Lower,
    Smooth,
    Flatten,
}

/// Что нажато в этом кадре: инструменту нужны позиция курсора и состояние кнопки.
#[derive(Clone, Copy, Default)]
pub struct Pointer {
    pub world: Option<Vec2>,
    pub pressed: bool,
    pub down: bool,
    pub released: bool,
    /// Ctrl — обратное действие (стереть, опустить, удалить точку).
    pub ctrl: bool,
    /// Shift — сгладить рельеф.
    pub shift: bool,
}

/// Содержимое чанка до мазка.
struct ChunkBackup {
    solid: Vec<u8>,
    ground: Vec<u8>,
    heights: Vec<u16>,
}

/// Один отменяемый шаг.
#[derive(Default)]
struct Stroke {
    chunks: HashMap<usize, ChunkBackup>,
    layout: Option<Layout>,
}

impl Stroke {
    fn is_empty(&self) -> bool {
        self.chunks.is_empty() && self.layout.is_none()
    }
}

/// Что сейчас тащим инструментом «Точки».
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Handle {
    Point(usize),
    Base(usize),
}

pub struct MapEditor {
    pub file: MapFile,
    pub dirty: bool,
    pub tool: Tool,
    pub wall_shape: WallShape,
    /// Прочность, которой рисуем стены.
    pub wall_kind: u8,
    /// Прямоугольник рисуется с дверями посередине каждой стороны.
    pub doors: bool,
    pub terrain_mode: TerrainMode,
    pub ground_material: u8,
    pub radius: f32,
    /// Для рельефа — метров в секунду.
    pub strength: f32,
    /// Радиус точки захвата — только для подсветки в редакторе.
    pub capture_radius: f32,
    pub issues: Vec<String>,
    undo: Vec<Stroke>,
    redo: Vec<Stroke>,
    stroke: Option<Stroke>,
    drag_from: Option<Vec2>,
    handle: Option<Handle>,
    flatten_to: f32,
}

impl MapEditor {
    pub fn new(file: MapFile, capture_radius: f32) -> Self {
        let mut editor = MapEditor {
            file,
            dirty: false,
            tool: Tool::Walls,
            wall_shape: WallShape::Brush,
            wall_kind: HOUSE_WALL,
            doors: true,
            terrain_mode: TerrainMode::Raise,
            ground_material: 2,
            radius: 3.0,
            strength: 2.0,
            capture_radius,
            issues: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            stroke: None,
            drag_from: None,
            handle: None,
            flatten_to: 0.0,
        };
        editor.validate();
        editor
    }

    /// Пустая карта: трава, ровная земля, скалы по краю, две базы и одна точка в центре.
    pub fn blank(name: &str, size: u32) -> MapFile {
        let mut map = Map::new(size, size);
        let n = size as i32;
        for i in 0..n {
            map.set(i, 0, INDESTRUCTIBLE);
            map.set(i, n - 1, INDESTRUCTIBLE);
            map.set(0, i, INDESTRUCTIBLE);
            map.set(n - 1, i, INDESTRUCTIBLE);
        }
        let s = size as f32;
        let margin = (s * 0.05).max(16.0);
        MapFile {
            name: name.to_owned(),
            map,
            layout: Layout {
                bases: [Vec2::new(margin, s * 0.5), Vec2::new(s - margin, s * 0.5)],
                capture_points: vec![Vec2::splat(s * 0.5)],
            },
        }
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    pub fn save(&mut self, maps_dir: &Path) -> Result<PathBuf, String> {
        let dir = maps_dir.join(&self.file.name);
        self.file.save(&dir)?;
        self.dirty = false;
        Ok(dir)
    }

    // --- Ввод -----------------------------------------------------------------

    /// Применяет инструмент. Вызывать каждый кадр, пока курсор над вьюпортом.
    pub fn handle(&mut self, p: Pointer, dt: f32) {
        if p.pressed
            && let Some(at) = p.world
        {
            self.stroke = Some(Stroke::default());
            self.drag_from = Some(at);
            self.flatten_to = self.file.map.height_at(at);
            if self.tool == Tool::Points {
                self.press_points(at, p.ctrl);
            }
        }
        if p.down
            && self.stroke.is_some()
            && let Some(at) = p.world
        {
            match self.tool {
                Tool::Walls if self.wall_shape == WallShape::Brush => {
                    let hp = if p.ctrl { EMPTY } else { self.wall_kind };
                    self.paint_cells(at, |m, i| {
                        let changed = m.solid[i] != hp;
                        m.solid[i] = hp;
                        changed
                    });
                }
                Tool::Ground => {
                    let material = self.ground_material;
                    self.paint_cells(at, |m, i| {
                        let changed = m.ground[i] != material;
                        m.ground[i] = material;
                        changed
                    });
                }
                Tool::Terrain => self.sculpt(at, p, dt),
                Tool::Points => self.drag_handle(at),
                Tool::Walls => {}
            }
        }
        if p.released {
            if self.tool == Tool::Walls
                && self.wall_shape != WallShape::Brush
                && let (Some(from), Some(to)) = (self.drag_from, p.world)
            {
                let hp = if p.ctrl { EMPTY } else { self.wall_kind };
                match self.wall_shape {
                    WallShape::Line => self.line(from, to, hp),
                    WallShape::Rect => self.rect(from, to, hp),
                    WallShape::Brush => {}
                }
            }
            self.finish_stroke();
        }
    }

    /// Начало линии или прямоугольника, пока кнопка зажата.
    pub fn drag_preview(&self, p: &Pointer) -> Option<(Vec2, Vec2)> {
        (self.tool == Tool::Walls && self.wall_shape != WallShape::Brush && p.down)
            .then_some(())
            .and(self.drag_from.zip(p.world))
    }

    fn finish_stroke(&mut self) {
        self.drag_from = None;
        self.handle = None;
        if let Some(stroke) = self.stroke.take()
            && !stroke.is_empty()
        {
            self.undo.push(stroke);
            if self.undo.len() > 100 {
                self.undo.remove(0);
            }
            self.redo.clear();
            self.dirty = true;
            self.validate();
        }
    }

    // --- Стены и покрытие -------------------------------------------------------

    fn paint_cells(&mut self, at: Vec2, mut apply: impl FnMut(&mut Map, usize) -> bool) {
        let r = self.radius;
        let (x0, y0) = Map::cell_of(at - Vec2::splat(r));
        let (x1, y1) = Map::cell_of(at + Vec2::splat(r));
        let mut ground_changed = false;
        for y in y0..=y1 {
            for x in x0..=x1 {
                let center = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
                if center.distance(at) > r.max(0.5) {
                    continue;
                }
                self.set_cell(x, y, &mut apply, &mut ground_changed);
            }
        }
        if ground_changed {
            self.file.map.ground_revision += 1;
        }
    }

    fn set_cell(&mut self, x: i32, y: i32, apply: &mut impl FnMut(&mut Map, usize) -> bool, ground_changed: &mut bool) {
        let Some(i) = self.file.map.index(x, y) else { return };
        let chunk = self.file.map.chunk_of_cell(x, y);
        self.backup(chunk);
        let before = self.file.map.ground[i];
        if apply(&mut self.file.map, i) {
            *ground_changed |= self.file.map.ground[i] != before;
            self.file.map.touch(x, y, true);
        }
    }

    fn line(&mut self, from: Vec2, to: Vec2, hp: u8) {
        let (mut x, mut y) = Map::cell_of(from);
        let (x1, y1) = Map::cell_of(to);
        let (dx, dy) = ((x1 - x).abs(), -(y1 - y).abs());
        let (sx, sy) = (if x < x1 { 1 } else { -1 }, if y < y1 { 1 } else { -1 });
        let mut err = dx + dy;
        let mut set = |m: &mut Map, i: usize| {
            let changed = m.solid[i] != hp;
            m.solid[i] = hp;
            changed
        };
        let mut unused = false;
        loop {
            self.set_cell(x, y, &mut set, &mut unused);
            if x == x1 && y == y1 {
                break;
            }
            let e2 = 2 * err;
            if e2 >= dy {
                err += dy;
                x += sx;
            }
            if e2 <= dx {
                err += dx;
                y += sy;
            }
        }
    }

    /// Контур дома; с дверями — проём в 2 клетки посередине каждой стороны.
    fn rect(&mut self, from: Vec2, to: Vec2, hp: u8) {
        let (ax, ay) = Map::cell_of(from);
        let (bx, by) = Map::cell_of(to);
        let (x0, x1, y0, y1) = (ax.min(bx), ax.max(bx), ay.min(by), ay.max(by));
        let (cx, cy) = ((x0 + x1) / 2, (y0 + y1) / 2);
        let door = |v: i32, c: i32, len: i32| len >= 5 && (v == c || v == c + 1);
        let mut set = |m: &mut Map, i: usize| {
            let changed = m.solid[i] != hp;
            m.solid[i] = hp;
            changed
        };
        let mut unused = false;
        for x in x0..=x1 {
            for y in [y0, y1] {
                if !(self.doors && door(x, cx, x1 - x0)) {
                    self.set_cell(x, y, &mut set, &mut unused);
                }
            }
        }
        for y in y0..=y1 {
            for x in [x0, x1] {
                if !(self.doors && door(y, cy, y1 - y0)) {
                    self.set_cell(x, y, &mut set, &mut unused);
                }
            }
        }
    }

    // --- Рельеф ----------------------------------------------------------------

    fn sculpt(&mut self, at: Vec2, p: Pointer, dt: f32) {
        let mode = if p.shift {
            TerrainMode::Smooth
        } else if p.ctrl && self.terrain_mode == TerrainMode::Raise {
            TerrainMode::Lower
        } else if p.ctrl && self.terrain_mode == TerrainMode::Lower {
            TerrainMode::Raise
        } else {
            self.terrain_mode
        };
        let r = self.radius.max(1.0);
        let (x0, y0) = ((at.x - r).floor() as i32, (at.y - r).floor() as i32);
        let (x1, y1) = ((at.x + r).ceil() as i32, (at.y + r).ceil() as i32);
        // Сначала считаем новые высоты, потом записываем — иначе сглаживание
        // зависит от порядка обхода.
        let mut updates = Vec::new();
        for y in y0..=y1 {
            for x in x0..=x1 {
                let d = Vec2::new(x as f32, y as f32).distance(at);
                if d > r || self.file.map.corner_index(x, y).is_none() {
                    continue;
                }
                let falloff = {
                    let t = 1.0 - d / r;
                    t * t * (3.0 - 2.0 * t)
                };
                let h = self.file.map.corner_height(x, y);
                let k = (self.strength * dt * falloff).min(1.0);
                let new = match mode {
                    TerrainMode::Raise => h + self.strength * dt * falloff,
                    TerrainMode::Lower => h - self.strength * dt * falloff,
                    TerrainMode::Smooth => {
                        let m = &self.file.map;
                        let avg = (m.corner_height(x - 1, y)
                            + m.corner_height(x + 1, y)
                            + m.corner_height(x, y - 1)
                            + m.corner_height(x, y + 1))
                            / 4.0;
                        h + (avg - h) * k
                    }
                    TerrainMode::Flatten => h + (self.flatten_to - h) * k,
                };
                updates.push((x, y, new.clamp(0.0, 200.0)));
            }
        }
        for (x, y, h) in updates {
            // Угол принадлежит четырём клеткам — запоминаем и отмечаем все их чанки.
            for (cx, cy) in [(x - 1, y - 1), (x, y - 1), (x - 1, y), (x, y)] {
                if self.file.map.index(cx, cy).is_some() {
                    let chunk = self.file.map.chunk_of_cell(cx, cy);
                    self.backup(chunk);
                }
            }
            self.file.map.set_corner_height(x, y, h);
            for (cx, cy) in [(x - 1, y - 1), (x, y - 1), (x - 1, y), (x, y)] {
                self.file.map.touch(cx, cy, false);
            }
        }
    }

    // --- Точки и базы --------------------------------------------------------------

    fn near_handle(&self, at: Vec2) -> Option<Handle> {
        let l = &self.file.layout;
        let point = l
            .capture_points
            .iter()
            .enumerate()
            .map(|(i, p)| (i, p.distance(at)))
            .filter(|&(_, d)| d < (self.capture_radius * 0.5).max(3.0))
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(i, _)| Handle::Point(i));
        let base = (0..2).find(|&i| l.bases[i].distance(at) < 7.0).map(Handle::Base);
        base.or(point)
    }

    fn press_points(&mut self, at: Vec2, ctrl: bool) {
        let near = self.near_handle(at);
        self.backup_layout();
        match (near, ctrl) {
            (Some(Handle::Point(i)), true) => {
                self.file.layout.capture_points.remove(i);
            }
            (Some(h), false) => self.handle = Some(h),
            (None, false) if self.file.layout.capture_points.len() < MAX_POINTS => {
                self.file.layout.capture_points.push(at);
                self.handle = Some(Handle::Point(self.file.layout.capture_points.len() - 1));
            }
            _ => {}
        }
    }

    fn drag_handle(&mut self, at: Vec2) {
        let size = Vec2::new(self.file.map.width as f32, self.file.map.height as f32);
        let at = at.clamp(Vec2::splat(2.0), size - 2.0);
        match self.handle {
            Some(Handle::Point(i)) => self.file.layout.capture_points[i] = at,
            Some(Handle::Base(i)) => self.file.layout.bases[i] = at,
            None => {}
        }
    }

    // --- Отмена ----------------------------------------------------------------

    fn backup(&mut self, chunk: usize) {
        let Some(stroke) = &mut self.stroke else { return };
        if stroke.chunks.contains_key(&chunk) {
            return;
        }
        let m = &self.file.map;
        let (x0, y0, w, h) = m.chunk_rect(chunk);
        let mut b = ChunkBackup { solid: Vec::new(), ground: Vec::new(), heights: Vec::new() };
        for y in y0..y0 + h {
            let row = (y * m.width + x0) as usize;
            b.solid.extend_from_slice(&m.solid[row..row + w as usize]);
            b.ground.extend_from_slice(&m.ground[row..row + w as usize]);
        }
        for y in y0..=y0 + h {
            let row = m.corner_index(x0 as i32, y as i32).unwrap();
            b.heights.extend_from_slice(&m.heights[row..row + w as usize + 1]);
        }
        stroke.chunks.insert(chunk, b);
    }

    fn backup_layout(&mut self) {
        if let Some(stroke) = &mut self.stroke
            && stroke.layout.is_none()
        {
            stroke.layout = Some(self.file.layout.clone());
        }
    }

    /// Меняет местами сохранённое и текущее — так одна функция и отменяет, и возвращает.
    fn swap(&mut self, stroke: &mut Stroke) {
        let m = &mut self.file.map;
        for (&chunk, b) in &mut stroke.chunks {
            let (x0, y0, w, h) = m.chunk_rect(chunk);
            for (dy, y) in (y0..y0 + h).enumerate() {
                let row = (y * m.width + x0) as usize;
                let part = dy * w as usize..(dy + 1) * w as usize;
                m.solid[row..row + w as usize].swap_with_slice(&mut b.solid[part.clone()]);
                m.ground[row..row + w as usize].swap_with_slice(&mut b.ground[part]);
            }
            for (dy, y) in (y0..=y0 + h).enumerate() {
                let row = m.corner_index(x0 as i32, y as i32).unwrap();
                let n = w as usize + 1;
                m.heights[row..row + n].swap_with_slice(&mut b.heights[dy * n..(dy + 1) * n]);
            }
            m.chunk_revisions[chunk] += 1;
            // Соседние чанки делят углы рельефа — перестраиваем и их.
            let cx = m.chunks_x() as i32;
            let (gx, gy) = ((chunk as i32) % cx, (chunk as i32) / cx);
            for (nx, ny) in [(gx - 1, gy), (gx + 1, gy), (gx, gy - 1), (gx, gy + 1)] {
                if nx >= 0 && ny >= 0 && nx < cx && ((ny * cx + nx) as usize) < m.chunk_revisions.len() {
                    m.chunk_revisions[(ny * cx + nx) as usize] += 1;
                }
            }
        }
        if !stroke.chunks.is_empty() {
            m.revision += 1;
            m.topology_revision += 1;
            m.ground_revision += 1;
        }
        if let Some(layout) = &mut stroke.layout {
            std::mem::swap(layout, &mut self.file.layout);
        }
    }

    pub fn undo(&mut self) {
        if let Some(mut s) = self.undo.pop() {
            self.swap(&mut s);
            self.redo.push(s);
            self.dirty = true;
            self.validate();
        }
    }

    pub fn redo(&mut self) {
        if let Some(mut s) = self.redo.pop() {
            self.swap(&mut s);
            self.undo.push(s);
            self.dirty = true;
            self.validate();
        }
    }

    // --- Проверка ----------------------------------------------------------------

    /// Что мешает играть на карте. Пустой список — всё в порядке.
    pub fn validate(&mut self) {
        let m = &self.file.map;
        let l = &self.file.layout;
        let mut issues = Vec::new();
        let names = ["синих", "красных"];
        if l.capture_points.is_empty() {
            issues.push("Нет ни одной точки захвата".into());
        }
        for (i, base) in l.bases.iter().enumerate() {
            if m.circle_hits_solid(*base, 1.0) {
                issues.push(format!("База {} стоит в стене", names[i]));
            }
        }
        for (i, p) in l.capture_points.iter().enumerate() {
            let letter = (b'A' + i as u8) as char;
            if m.circle_hits_solid(*p, 1.0) {
                issues.push(format!("Точка {letter} стоит в стене"));
                continue;
            }
            let mut field = FlowField::new(*p);
            field.compute(m);
            for (team, base) in l.bases.iter().enumerate() {
                let (bx, by) = Map::cell_of(*base);
                let reachable = m.index(bx, by).is_some_and(|c| field.dist[c] != UNREACHABLE);
                if !reachable {
                    issues.push(format!("До точки {letter} не дойти от базы {}", names[team]));
                }
            }
        }
        self.issues = issues;
    }
}

/// Сколько чанков по стороне — для подписи в свойствах карты.
pub fn chunks_per_side(size: u32) -> u32 {
    size.div_ceil(CHUNK)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn click(editor: &mut MapEditor, at: Vec2, ctrl: bool) {
        let p = Pointer { world: Some(at), pressed: true, down: true, released: false, ctrl, shift: false };
        editor.handle(p, 0.1);
        editor.handle(Pointer { pressed: false, down: false, released: true, ..p }, 0.1);
    }

    #[test]
    fn undo_and_redo_walls_and_terrain() {
        let mut e = MapEditor::new(MapEditor::blank("t", 128), 10.0);
        let before = e.file.map.solid.clone();
        click(&mut e, Vec2::new(40.5, 40.5), false);
        assert_ne!(e.file.map.solid, before);
        e.tool = Tool::Terrain;
        let heights = e.file.map.heights.clone();
        click(&mut e, Vec2::new(64.0, 64.0), false);
        assert_ne!(e.file.map.heights, heights);
        e.undo();
        assert_eq!(e.file.map.heights, heights);
        e.undo();
        assert_eq!(e.file.map.solid, before);
        e.redo();
        assert_ne!(e.file.map.solid, before);
    }

    #[test]
    fn rect_has_doors_and_validation_sees_walled_point() {
        let mut e = MapEditor::new(MapEditor::blank("t", 128), 10.0);
        assert!(e.issues.is_empty(), "{:?}", e.issues);
        e.wall_shape = WallShape::Rect;
        e.wall_kind = INDESTRUCTIBLE;
        // Глухая коробка вокруг центральной точки.
        e.doors = false;
        let press = Pointer { world: Some(Vec2::new(54.0, 54.0)), pressed: true, down: true, ..Default::default() };
        e.handle(press, 0.1);
        e.handle(Pointer { world: Some(Vec2::new(74.0, 74.0)), released: true, ..Default::default() }, 0.1);
        assert!(e.issues.iter().any(|i| i.contains("не дойти")), "{:?}", e.issues);
        e.undo();
        assert!(e.issues.is_empty());
    }

    #[test]
    fn points_add_move_delete() {
        let mut e = MapEditor::new(MapEditor::blank("t", 128), 10.0);
        e.tool = Tool::Points;
        click(&mut e, Vec2::new(30.0, 90.0), false);
        assert_eq!(e.file.layout.capture_points.len(), 2);
        click(&mut e, Vec2::new(30.0, 90.0), true);
        assert_eq!(e.file.layout.capture_points.len(), 1);
        e.undo();
        assert_eq!(e.file.layout.capture_points.len(), 2);
    }
}
