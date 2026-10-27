//! Рабочее пространство «Редактор карт»: полка инструментов слева,
//! свойства справа, подсказки снизу — как в Blender.

use crate::editor::{FENCE, HOUSE_WALL, MapEditor, Pointer, TerrainMode, Tool, WallShape};
use wind_sim::prefab::{self, Prefab};
use crate::icons::{self, Icon};
use crate::theme;
use crate::viewport::{Scene, ScenePoint, ViewportOutput};
use eframe::egui::{self, Color32, RichText};
use std::path::PathBuf;
use wind_sim::map::{GROUND_NAMES, INDESTRUCTIBLE};
use wind_sim::atmosphere::{Atmosphere, Preset};
use wind_sim::mapfile::MapFile;
use wind_sim::{SimConfig, Vec2};

/// Цвета покрытия в 2D-виде (sRGB). Порядок — как в `GROUND_NAMES`.
pub const GROUND_2D: [Color32; 6] = [
    Color32::from_rgb(78, 92, 58),
    Color32::from_rgb(110, 106, 72),
    Color32::from_rgb(102, 86, 64),
    Color32::from_rgb(170, 156, 116),
    Color32::from_rgb(66, 66, 70),
    Color32::from_rgb(140, 138, 132),
];

/// Действие, которое потеряет несохранённые изменения, — ждёт подтверждения.
#[derive(Clone, PartialEq)]
enum Pending {
    New,
    Open(String),
}

/// Что редактор просит у лаборатории.
pub enum Request {
    /// Запустить симуляцию на этой карте.
    Play(MapFile),
}

pub struct MapWorkspace {
    pub editor: MapEditor,
    maps_dir: PathBuf,
    new_name: String,
    new_size: u32,
    generate: bool,
    new_seed: u64,
    new_points: u32,
    open_choice: String,
    pending: Option<Pending>,
    pub message: (String, Color32),
    pointer: Pointer,
    /// Растёт при открытии другой карты — вьюпорт должен перестроить меши.
    pub generation: u64,
    /// Готовые здания: встроенные и свои из `data/prefabs.ron`.
    builtin: Vec<Prefab>,
    user: Vec<Prefab>,
    /// Имя, под которым сохранить выделенное как здание.
    prefab_name: String,
}

impl MapWorkspace {
    pub fn new(maps_dir: PathBuf, config: &SimConfig) -> Self {
        let file = MapFile::generated("новая карта", 256, 3, 1);
        MapWorkspace {
            editor: MapEditor::new(file, config.capture.radius),
            maps_dir,
            new_name: "моя карта".into(),
            new_size: 512,
            generate: false,
            new_seed: 1,
            new_points: 5,
            open_choice: String::new(),
            pending: None,
            message: (String::new(), theme::TEXT_DIM),
            pointer: Pointer::default(),
            generation: 0,
            builtin: prefab::builtin(),
            user: Vec::new(),
            prefab_name: "моё здание".into(),
        }
        .with_user_prefabs()
    }

    /// Свои здания лежат рядом с картами: `data/prefabs.ron`.
    fn prefabs_path(&self) -> PathBuf {
        self.maps_dir.parent().unwrap_or(&self.maps_dir).join("prefabs.ron")
    }

    fn with_user_prefabs(mut self) -> Self {
        match prefab::load(&self.prefabs_path()) {
            Ok(list) => self.user = list,
            Err(e) => self.error(e),
        }
        self
    }

    /// Выделенное — в свои здания, в `data/prefabs.ron`.
    fn save_selection_as_prefab(&mut self) {
        let name = self.prefab_name.trim().to_owned();
        let Some(mut p) = self.editor.selection_prefab(&name) else { return };
        // Клетки без стен и с обычной землёй вокруг здания — «не трогать», чтобы
        // здание не стирало траву там, куда его поставят.
        p = trim_background(&p);
        self.user.retain(|u| u.name != name);
        self.user.push(p.clone());
        match prefab::save(&self.prefabs_path(), &self.user) {
            Ok(()) => {
                self.editor.stamp = Some(p);
                self.info(format!("Здание «{name}» сохранено в {}", self.prefabs_path().display()));
            }
            Err(e) => self.error(e),
        }
    }

    fn info(&mut self, text: String) {
        self.message = (text, theme::TEXT_DIM);
    }

    fn error(&mut self, text: String) {
        self.message = (text, theme::ERROR);
    }

    fn replace(&mut self, file: MapFile) {
        let radius = self.editor.capture_radius;
        self.editor = MapEditor::new(file, radius);
        self.generation += 1;
    }

    fn save(&mut self) {
        match self.editor.save(&self.maps_dir) {
            Ok(dir) => self.info(format!("Сохранено: {}", dir.display())),
            Err(e) => self.error(e),
        }
    }

    fn run(&mut self, action: Pending) {
        if self.editor.dirty && self.pending.as_ref() != Some(&action) {
            // Первый клик только предупреждает.
            self.pending = Some(action);
            self.error("Есть несохранённые изменения — нажми ещё раз, чтобы продолжить без сохранения".into());
            return;
        }
        self.pending = None;
        match action {
            Pending::New => {
                let name = self.new_name.trim().to_owned();
                let file = if self.generate {
                    MapFile::generated(&name, self.new_size, self.new_points, self.new_seed)
                } else {
                    MapEditor::blank(&name, self.new_size)
                };
                self.replace(file);
                self.info(format!("Новая карта «{name}», {0}×{0} м", self.new_size));
            }
            Pending::Open(name) => match MapFile::load(&self.maps_dir.join(&name)) {
                Ok(file) => {
                    self.replace(file);
                    self.info(format!("Открыта карта «{name}»"));
                }
                Err(e) => self.error(e),
            },
        }
    }

    pub fn maps_dir(&self) -> &std::path::Path {
        &self.maps_dir
    }

    /// Открыть карту из папки карт по имени.
    pub fn open(&mut self, name: &str) {
        self.run(Pending::Open(name.to_owned()));
    }

    // --- Ввод -----------------------------------------------------------------

    /// Горячие клавиши. Вызывать, когда редактор на экране.
    pub fn shortcuts(&mut self, ctx: &egui::Context) {
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        let (undo, redo, save, smaller, bigger, tool) = ctx.input(|i| {
            let ctrl = i.modifiers.command;
            let tool = [
                (egui::Key::Num1, Tool::Walls),
                (egui::Key::Num2, Tool::Terrain),
                (egui::Key::Num3, Tool::Ground),
                (egui::Key::Num4, Tool::Points),
                (egui::Key::Num5, Tool::Select),
                (egui::Key::Num6, Tool::Buildings),
            ]
            .into_iter()
            .find(|(k, _)| i.key_pressed(*k))
            .map(|(_, t)| t);
            (
                ctrl && !i.modifiers.shift && i.key_pressed(egui::Key::Z),
                ctrl && ((i.modifiers.shift && i.key_pressed(egui::Key::Z)) || i.key_pressed(egui::Key::Y)),
                ctrl && i.key_pressed(egui::Key::S),
                i.key_pressed(egui::Key::OpenBracket),
                i.key_pressed(egui::Key::CloseBracket),
                tool,
            )
        });
        let (copy, paste, delete, rotate, escape) = ctx.input(|i| {
            let ctrl = i.modifiers.command;
            (
                ctrl && i.key_pressed(egui::Key::C),
                ctrl && i.key_pressed(egui::Key::V),
                i.key_pressed(egui::Key::Delete),
                !ctrl && i.key_pressed(egui::Key::R),
                i.key_pressed(egui::Key::Escape),
            )
        });
        if copy && self.editor.copy() {
            self.info("Скопировано: Ctrl+V — вставить".into());
        }
        if paste && !self.editor.paste() {
            self.error("Буфер пуст: выдели кусок карты и нажми Ctrl+C".into());
        }
        if delete && self.editor.tool == Tool::Select {
            self.editor.delete_selection();
        }
        if rotate && self.editor.tool == Tool::Buildings {
            self.editor.rotate_stamp();
        }
        if escape {
            self.editor.selection = None;
        }
        if undo {
            self.editor.undo();
        }
        if redo {
            self.editor.redo();
        }
        if save {
            self.save();
        }
        if smaller {
            self.editor.radius = (self.editor.radius / 1.25).max(0.5);
        }
        if bigger {
            self.editor.radius = (self.editor.radius * 1.25).min(60.0);
        }
        if let Some(t) = tool {
            self.editor.tool = t;
        }
    }

    /// Применяет инструмент по тому, что произошло во вьюпорте.
    pub fn handle(&mut self, ctx: &egui::Context, out: &ViewportOutput) {
        let (ctrl, shift, dt) = ctx.input(|i| (i.modifiers.command, i.modifiers.shift, i.stable_dt.min(0.1)));
        self.pointer = Pointer {
            world: out.mouse_world,
            pressed: out.primary_pressed,
            down: out.primary_down,
            released: out.primary_released,
            ctrl,
            shift,
        };
        self.editor.handle(self.pointer, dt);
    }

    // --- Отрисовка -----------------------------------------------------------------

    pub fn scene<'a>(&'a self, config: &SimConfig) -> Scene<'a> {
        let e = &self.editor;
        let file = &e.file;
        let mut rings = Vec::new();
        let mut extra = Vec::new();
        // Выделение — рамкой.
        if let Some(s) = e.selection {
            extra.extend(frame(s.x0, s.y0, s.width(), s.height()));
        }
        // Здание под курсором: контур и стены клетками.
        if e.tool == Tool::Buildings
            && let (Some(at), Some(stamp)) = (self.pointer.world, &e.stamp)
            && let Some((x0, y0)) = e.stamp_origin(at)
        {
            extra.extend(frame(x0, y0, stamp.width, stamp.height));
            for y in 0..stamp.height {
                for x in 0..stamp.width {
                    if stamp.cell(x, y).wall.is_some_and(|w| w != 0) {
                        extra.extend(frame(x0 + x, y0 + y, 1, 1));
                    }
                }
            }
            rings.push((at, 0.6, [1.0, 1.0, 1.0]));
        }
        if let Some(at) = self.pointer.world
            && !matches!(e.tool, Tool::Points | Tool::Select | Tool::Buildings)
        {
            let erase = self.pointer.ctrl && matches!(e.tool, Tool::Walls);
            let color = if erase { [1.0, 0.45, 0.4] } else { [1.0, 1.0, 1.0] };
            let radius = if e.tool == Tool::Walls && e.wall_shape != WallShape::Brush { 0.6 } else { e.radius };
            rings.push((at, radius, color));
        }
        let lines = e.drag_preview(&self.pointer).map(|(a, b)| {
            if e.wall_shape == WallShape::Rect {
                let (c, d) = (Vec2::new(b.x, a.y), Vec2::new(a.x, b.y));
                vec![(a, c), (c, b), (b, d), (d, a)]
            } else {
                vec![(a, b)]
            }
        });
        Scene {
            title: format!("Редактор карт · {}{}", file.name, if e.dirty { " *" } else { "" }),
            map: &file.map,
            bases: file.layout.bases,
            points: file
                .layout
                .capture_points
                .iter()
                .map(|&pos| ScenePoint { pos, owner: None, progress: 0.0 })
                .collect(),
            capture_radius: e.capture_radius,
            soldier_radius: config.soldier.radius,
            soldiers: Vec::new(),
            bullets: Vec::new(),
            grenades: Vec::new(),
            blasts: Vec::new(),
            selected: None,
            flow: None,
            hud: None,
            kill_feed: Vec::new(),
            rings,
            lines: lines.unwrap_or_default().into_iter().chain(extra).collect(),
            corpses: Vec::new(),
            atmosphere: file.atmosphere.clone(),
        }
    }

    pub fn hint(&self) -> &'static str {
        match self.editor.tool {
            Tool::Walls => match self.editor.wall_shape {
                WallShape::Brush => "ЛКМ — рисовать стены · Ctrl — стирать · [ ] — размер кисти · Ctrl+Z — отмена · Ctrl+S — сохранить",
                WallShape::Line => "Тяни ЛКМ — линия стены · Ctrl — стереть линией · Ctrl+Z — отмена",
                WallShape::Rect => "Тяни ЛКМ — дом из угла в угол · Ctrl — стереть контур · Ctrl+Z — отмена",
            },
            Tool::Terrain => "ЛКМ — менять рельеф · Ctrl — наоборот · Shift — сгладить · [ ] — размер кисти",
            Tool::Ground => "ЛКМ — красить землю · [ ] — размер кисти · Ctrl+Z — отмена",
            Tool::Points => "ЛКМ по пустому месту — новая точка · тяни точку или базу — двигать · Ctrl+ЛКМ — удалить точку",
            Tool::Select => "Тяни ЛКМ — выделить · Ctrl+C — копировать · Ctrl+V — вставить · Delete — стереть стены · Esc — снять",
            Tool::Buildings => "ЛКМ — поставить здание · R — повернуть · Ctrl+V — вставить скопированное · Ctrl+Z — отмена",
        }
    }

    /// Полка инструментов слева (как T-панель в Blender) и список объектов карты.
    pub fn tool_shelf(&mut self, ui: &mut egui::Ui) -> Option<Vec2> {
        ui.label(RichText::new("Инструменты").strong());
        ui.add_space(2.0);
        let tools = [
            ("1", Tool::Walls, Icon::Walls, "Стены"),
            ("2", Tool::Terrain, Icon::Terrain, "Рельеф"),
            ("3", Tool::Ground, Icon::Ground, "Покрытие"),
            ("4", Tool::Points, Icon::Flag, "Точки и базы"),
            ("5", Tool::Select, Icon::Select, "Выделение"),
            ("6", Tool::Buildings, Icon::Building, "Здания"),
        ];
        for (key, tool, icon, name) in tools {
            let text = format!("{name}   {key}");
            let r = ui.add_sized([ui.available_width(), 26.0], egui::Button::selectable(self.editor.tool == tool, text.as_str()));
            if icons::decorate_centered(ui, r, icon, &text).clicked() {
                self.editor.tool = tool;
            }
        }
        ui.add_space(10.0);
        ui.label(RichText::new("Сцена").strong());
        ui.separator();
        let mut focus = None;
        let l = &self.editor.file.layout;
        for (i, base) in l.bases.iter().enumerate() {
            if icons::selectable(ui, false, Icon::Team, &format!("База {}", ["синих", "красных"][i]), Some(theme::TEAM[i])).clicked() {
                focus = Some(*base);
            }
        }
        for (i, p) in l.capture_points.iter().enumerate() {
            if icons::selectable(ui, false, Icon::Flag, &format!("Точка {}", (b'A' + i as u8) as char), None).clicked() {
                focus = Some(*p);
            }
        }
        focus
    }

    /// Свойства справа. Возвращает просьбу к лаборатории, если она есть.
    pub fn properties(&mut self, ui: &mut egui::Ui) -> Option<Request> {
        let mut request = None;
        theme::panel(ui, Icon::Wrench, "Инструмент", true, |ui| self.tool_panel(ui));
        theme::panel(ui, Icon::MapEditor, "Карта", true, |ui| request = self.map_panel(ui));
        theme::panel(ui, Icon::Sun, "Атмосфера", true, |ui| self.atmosphere_panel(ui));
        theme::panel(ui, Icon::Check, "Проверка", true, |ui| {
            if self.editor.issues.is_empty() {
                icons::label(ui, Icon::Check, "Всё в порядке: на карте можно играть", Color32::from_rgb(120, 200, 110));
            }
            for issue in &self.editor.issues {
                icons::label(ui, Icon::Warning, issue, theme::ERROR);
            }
        });
        theme::panel(ui, Icon::New, "Новая карта", false, |ui| self.new_panel(ui));
        theme::panel(ui, Icon::Open, "Открыть", true, |ui| self.open_panel(ui));
        if !self.message.0.is_empty() {
            ui.label(RichText::new(&self.message.0).small().color(self.message.1));
        }
        request
    }

    fn tool_panel(&mut self, ui: &mut egui::Ui) {
        match self.editor.tool {
            Tool::Select => return self.select_panel(ui),
            Tool::Buildings => return self.buildings_panel(ui),
            _ => {}
        }
        let e = &mut self.editor;
        theme::props(ui, "tool_props", |ui| match e.tool {
            Tool::Walls => {
                theme::label(ui, "Форма");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut e.wall_shape, WallShape::Brush, "Кисть");
                    ui.selectable_value(&mut e.wall_shape, WallShape::Line, "Линия");
                    ui.selectable_value(&mut e.wall_shape, WallShape::Rect, "Дом");
                });
                ui.end_row();
                theme::label(ui, "Материал");
                ui.horizontal(|ui| {
                    ui.selectable_value(&mut e.wall_kind, HOUSE_WALL, "Стена");
                    ui.selectable_value(&mut e.wall_kind, FENCE, "Забор");
                    ui.selectable_value(&mut e.wall_kind, INDESTRUCTIBLE, "Скала");
                });
                ui.end_row();
                if e.wall_shape == WallShape::Rect {
                    theme::label(ui, "Двери");
                    ui.checkbox(&mut e.doors, "по одной на сторону");
                    ui.end_row();
                }
                if e.wall_shape == WallShape::Brush {
                    theme::label(ui, "Радиус");
                    ui.add(egui::Slider::new(&mut e.radius, 0.5..=20.0).suffix(" м"));
                    ui.end_row();
                }
            }
            Tool::Terrain => {
                theme::label(ui, "Режим");
                egui::ComboBox::from_id_salt("terrain_mode")
                    .selected_text(terrain_name(e.terrain_mode))
                    .show_ui(ui, |ui| {
                        for m in [TerrainMode::Raise, TerrainMode::Lower, TerrainMode::Smooth, TerrainMode::Flatten] {
                            ui.selectable_value(&mut e.terrain_mode, m, terrain_name(m));
                        }
                    });
                ui.end_row();
                theme::label(ui, "Радиус");
                ui.add(egui::Slider::new(&mut e.radius, 1.0..=60.0).suffix(" м"));
                ui.end_row();
                theme::label(ui, "Сила");
                ui.add(egui::Slider::new(&mut e.strength, 0.2..=10.0).suffix(" м/с"));
                ui.end_row();
            }
            Tool::Ground => {
                theme::label(ui, "Покрытие");
                ui.vertical(|ui| {
                    for (i, name) in GROUND_NAMES.iter().enumerate() {
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
                            ui.painter().rect_filled(r, 3, GROUND_2D[i]);
                            ui.selectable_value(&mut e.ground_material, i as u8, *name);
                        });
                    }
                });
                ui.end_row();
                theme::label(ui, "Радиус");
                ui.add(egui::Slider::new(&mut e.radius, 0.5..=40.0).suffix(" м"));
                ui.end_row();
            }
            Tool::Points => {
                theme::label(ui, "Точек");
                ui.label(format!("{} из {}", e.file.layout.capture_points.len(), crate::editor::MAX_POINTS));
                ui.end_row();
            }
            Tool::Select | Tool::Buildings => {}
        });
        ui.label(RichText::new(self.hint()).small().color(theme::TEXT_DIM));
    }

    fn select_panel(&mut self, ui: &mut egui::Ui) {
        let Some(s) = self.editor.selection else {
            ui.label(RichText::new("Протяни мышью по карте, чтобы выделить.").color(theme::TEXT_DIM));
            ui.label(RichText::new(self.hint()).small().color(theme::TEXT_DIM));
            return;
        };
        ui.label(format!("Выделено {}×{} м", s.width(), s.height()));
        ui.horizontal(|ui| {
            if ui.button("Копировать  Ctrl+C").clicked() && self.editor.copy() {
                self.info("Скопировано: Ctrl+V — вставить".into());
            }
            if ui.button("Стереть стены  Del").clicked() {
                self.editor.delete_selection();
            }
        });
        ui.add_space(4.0);
        ui.label(RichText::new("Сохранить как здание").strong());
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.prefab_name).desired_width(150.0));
            let ok = !self.prefab_name.trim().is_empty() && s.width() <= 64 && s.height() <= 64;
            if icons::button_enabled(ui, ok, Icon::Save, "Сохранить").clicked() {
                self.save_selection_as_prefab();
            }
        });
        ui.label(RichText::new("Попадёт в data/prefabs.ron — его можно поправить и руками.").small().color(theme::TEXT_DIM));
        ui.label(RichText::new(self.hint()).small().color(theme::TEXT_DIM));
    }

    fn buildings_panel(&mut self, ui: &mut egui::Ui) {
        let current = self.editor.stamp.as_ref().map(|p| p.name.clone());
        let mut chosen = None;
        let mut list = |ui: &mut egui::Ui, title: &str, items: &[Prefab]| {
            if items.is_empty() {
                return;
            }
            ui.label(RichText::new(title).strong());
            for p in items {
                let text = format!("{}   {}×{}", p.name, p.width, p.height);
                if ui.selectable_label(current.as_deref() == Some(p.name.as_str()), text).clicked() {
                    chosen = Some(p.clone());
                }
            }
        };
        list(ui, "Встроенные", &self.builtin);
        list(ui, "Свои (data/prefabs.ron)", &self.user);
        if let Some(clip) = &self.editor.clipboard
            && ui.selectable_label(current.as_deref() == Some("Копия"), format!("Из буфера   {}×{}", clip.width, clip.height)).clicked()
        {
            chosen = Some(clip.clone());
        }
        if let Some(p) = chosen {
            self.editor.stamp = Some(p);
        }
        if let Some(stamp) = &self.editor.stamp {
            ui.add_space(4.0);
            prefab_preview(ui, stamp);
            if ui.button("Повернуть  R").clicked() {
                self.editor.rotate_stamp();
            }
        }
        ui.label(RichText::new(self.hint()).small().color(theme::TEXT_DIM));
    }

    fn map_panel(&mut self, ui: &mut egui::Ui) -> Option<Request> {
        let mut request = None;
        let size = self.editor.file.map.width;
        theme::props(ui, "map_props", |ui| {
            theme::label(ui, "Имя");
            if ui.add(egui::TextEdit::singleline(&mut self.editor.file.name).desired_width(160.0)).changed() {
                self.editor.dirty = true;
            }
            ui.end_row();
            theme::label(ui, "Размер");
            ui.label(format!("{size}×{size} м, чанков {0}×{0}", crate::editor::chunks_per_side(size)));
            ui.end_row();
            theme::label(ui, "Отмена");
            ui.horizontal(|ui| {
                if icons::button_enabled(ui, self.editor.can_undo(), Icon::Undo, "Ctrl+Z").clicked() {
                    self.editor.undo();
                }
                if icons::button_enabled(ui, self.editor.can_redo(), Icon::Redo, "Ctrl+Shift+Z").clicked() {
                    self.editor.redo();
                }
            });
            ui.end_row();
        });
        ui.horizontal(|ui| {
            let save = if self.editor.dirty { "Сохранить *  (Ctrl+S)" } else { "Сохранить  (Ctrl+S)" };
            if icons::button(ui, Icon::Save, save).clicked() {
                self.save();
            }
            let playable = self.editor.issues.is_empty();
            if icons::button_enabled(ui, playable, Icon::Play, "Играть на этой карте").clicked() {
                request = Some(Request::Play(self.editor.file.clone()));
            }
        });
        ui.label(RichText::new(format!("Папка: {}", self.maps_dir.join(&self.editor.file.name).display())).small().color(theme::TEXT_DIM));
        request
    }

    /// Свет, туман и «плёнка» карты. Видно сразу в 3D-окне.
    fn atmosphere_panel(&mut self, ui: &mut egui::Ui) {
        let a = &mut self.editor.file.atmosphere;
        let mut changed = false;
        ui.horizontal_wrapped(|ui| {
            for p in Preset::ALL {
                if ui.button(p.name()).clicked() {
                    *a = Atmosphere::preset(p);
                    changed = true;
                }
            }
        });
        theme::props(ui, "atmosphere_props", |ui| {
            let mut slider = |ui: &mut egui::Ui, name: &str, v: &mut f32, range: std::ops::RangeInclusive<f32>| {
                theme::label(ui, name);
                changed |= ui.add(egui::Slider::new(v, range)).changed();
                ui.end_row();
            };
            slider(ui, "Солнце: азимут", &mut a.sun_azimuth, 0.0..=360.0);
            slider(ui, "Солнце: высота", &mut a.sun_elevation, 5.0..=90.0);
            slider(ui, "Солнце: сила", &mut a.sun_strength, 0.0..=3.0);
            slider(ui, "Туман", &mut a.fog, 0.0..=0.03);
            slider(ui, "Ступенчатость света", &mut a.toon, 0.0..=1.0);
            slider(ui, "Насыщенность", &mut a.saturation, 0.0..=1.5);
            slider(ui, "Контраст", &mut a.contrast, 0.5..=1.5);
            slider(ui, "Яркость", &mut a.brightness, 0.5..=1.5);
            slider(ui, "Обводка", &mut a.outline, 0.0..=1.0);
            slider(ui, "Затенение", &mut a.occlusion, 0.0..=1.0);
            slider(ui, "Виньетка", &mut a.vignette, 0.0..=1.0);
            let mut color = |ui: &mut egui::Ui, name: &str, c: &mut [f32; 3]| {
                theme::label(ui, name);
                changed |= ui.color_edit_button_rgb(c).changed();
                ui.end_row();
            };
            color(ui, "Цвет солнца", &mut a.sun_color);
            color(ui, "Цвет неба", &mut a.sky_color);
            color(ui, "Цвет тени", &mut a.shadow_color);
            color(ui, "Оттенок", &mut a.tint);
        });
        if changed {
            self.editor.dirty = true;
        }
    }

    fn new_panel(&mut self, ui: &mut egui::Ui) {
        theme::props(ui, "new_props", |ui| {
            theme::label(ui, "Имя");
            ui.add(egui::TextEdit::singleline(&mut self.new_name).desired_width(160.0));
            ui.end_row();
            theme::label(ui, "Размер");
            egui::ComboBox::from_id_salt("new_size")
                .selected_text(format!("{0}×{0} м", self.new_size))
                .show_ui(ui, |ui| {
                    for s in [128, 256, 512, 1024, 2048] {
                        ui.selectable_value(&mut self.new_size, s, format!("{s}×{s} м"));
                    }
                });
            ui.end_row();
            theme::label(ui, "Начать с");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.generate, false, "Пустой");
                ui.selectable_value(&mut self.generate, true, "Генерации");
            });
            ui.end_row();
            if self.generate {
                theme::label(ui, "Seed");
                ui.add(egui::DragValue::new(&mut self.new_seed));
                ui.end_row();
                theme::label(ui, "Точек захвата");
                ui.add(egui::DragValue::new(&mut self.new_points).range(1..=26));
                ui.end_row();
            }
        });
        if icons::wide_button(ui, Icon::New, "Создать").clicked() {
            self.run(Pending::New);
        }
    }

    fn open_panel(&mut self, ui: &mut egui::Ui) {
        let maps = MapFile::list(&self.maps_dir);
        if maps.is_empty() {
            ui.label(RichText::new("В папке data/maps пока нет карт.").color(theme::TEXT_DIM));
            return;
        }
        if !maps.contains(&self.open_choice) {
            self.open_choice = maps[0].clone();
        }
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("open_map").selected_text(&self.open_choice).show_ui(ui, |ui| {
                for m in &maps {
                    ui.selectable_value(&mut self.open_choice, m.clone(), m);
                }
            });
            if icons::button(ui, Icon::Open, "Открыть").clicked() {
                let name = self.open_choice.clone();
                self.run(Pending::Open(name));
            }
        });
    }
}

/// Контур клеток от (`x`, `y`) размером `w`×`h` — четыре отрезка.
fn frame(x: i32, y: i32, w: i32, h: i32) -> [(Vec2, Vec2); 4] {
    let (a, b) = (Vec2::new(x as f32, y as f32), Vec2::new((x + w) as f32, (y + h) as f32));
    let (c, d) = (Vec2::new(b.x, a.y), Vec2::new(a.x, b.y));
    [(a, c), (c, b), (b, d), (d, a)]
}

/// Клетки без стен с обычной землёй (трава, грунт, песок) — «не трогать».
fn trim_background(p: &Prefab) -> Prefab {
    let mut cells = Vec::with_capacity((p.width * p.height) as usize);
    for y in 0..p.height {
        for x in 0..p.width {
            let c = p.cell(x, y);
            let natural = c.wall == Some(0) && c.ground.is_some_and(|g| g < prefab::ASPHALT);
            cells.push(if natural { prefab::Cell::default() } else { c });
        }
    }
    Prefab::from_cells(&p.name, p.width, p.height, cells)
}

/// Маленькая картинка здания: стены цветом прочности, пол — серым.
fn prefab_preview(ui: &mut egui::Ui, p: &Prefab) {
    let cell = (180.0 / p.width.max(p.height) as f32).clamp(3.0, 12.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(p.width as f32 * cell, p.height as f32 * cell), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 2, theme::FIELD);
    for y in 0..p.height {
        for x in 0..p.width {
            let c = p.cell(x, y);
            let color = match (c.wall, c.ground) {
                (Some(w), _) if w == INDESTRUCTIBLE => Color32::from_gray(40),
                (Some(w), _) if w >= 160 => Color32::from_rgb(190, 190, 200),
                (Some(w), _) if w >= 80 => Color32::from_rgb(230, 220, 200),
                (Some(w), _) if w > 0 => Color32::from_rgb(160, 120, 70),
                (_, Some(g)) => GROUND_2D[g.min(5) as usize],
                // Пусто, земля как была — трава для примера.
                (Some(_), None) => GROUND_2D[0],
                (None, _) => continue,
            };
            let at = rect.min + egui::vec2(x as f32 * cell, y as f32 * cell);
            painter.rect_filled(egui::Rect::from_min_size(at, egui::vec2(cell, cell)), 0, color);
        }
    }
}

fn terrain_name(m: TerrainMode) -> &'static str {
    match m {
        TerrainMode::Raise => "Поднять",
        TerrainMode::Lower => "Опустить",
        TerrainMode::Smooth => "Сгладить",
        TerrainMode::Flatten => "Выровнять",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_building_does_not_carry_grass_around() {
        // Сарай на траве: вокруг — пустые клетки с травой, внутри — пол.
        let cells: Vec<prefab::Cell> = ["-----", "-###-", "-#.#-", "-----"]
            .concat()
            .chars()
            .map(|c| match c {
                '#' => prefab::Cell { wall: Some(120), ground: Some(0) },
                '.' => prefab::Cell { wall: Some(0), ground: Some(prefab::FLOOR) },
                _ => prefab::Cell { wall: Some(0), ground: Some(0) },
            })
            .collect();
        let p = trim_background(&Prefab::from_cells("сарай", 5, 4, cells));
        assert_eq!(p.rows(), vec!["     ", " ### ", " #.# ", "     "]);
    }
}
