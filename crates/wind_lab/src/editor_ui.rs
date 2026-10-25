//! Рабочее пространство «Редактор карт»: полка инструментов слева,
//! свойства справа, подсказки снизу — как в Blender.

use crate::editor::{FENCE, HOUSE_WALL, MapEditor, Pointer, TerrainMode, Tool, WallShape};
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
            let tool = [(egui::Key::Num1, Tool::Walls), (egui::Key::Num2, Tool::Terrain), (egui::Key::Num3, Tool::Ground), (egui::Key::Num4, Tool::Points)]
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
        if let Some(at) = self.pointer.world
            && e.tool != Tool::Points
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
            lines: lines.unwrap_or_default(),
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
        }
    }

    /// Полка инструментов слева (как T-панель в Blender) и список объектов карты.
    pub fn tool_shelf(&mut self, ui: &mut egui::Ui) -> Option<Vec2> {
        ui.label(RichText::new("Инструменты").strong());
        ui.add_space(2.0);
        for (key, tool, name) in [("1", Tool::Walls, "Стены"), ("2", Tool::Terrain, "Рельеф"), ("3", Tool::Ground, "Покрытие"), ("4", Tool::Points, "Точки и базы")] {
            let text = format!("{key}   {name}");
            if ui.add_sized([ui.available_width(), 26.0], egui::Button::selectable(self.editor.tool == tool, text)).clicked() {
                self.editor.tool = tool;
            }
        }
        ui.add_space(10.0);
        ui.label(RichText::new("Сцена").strong());
        ui.separator();
        let mut focus = None;
        let l = &self.editor.file.layout;
        for (i, base) in l.bases.iter().enumerate() {
            let label = RichText::new(format!("База {}", ["синих", "красных"][i])).color(theme::TEAM[i]);
            if ui.selectable_label(false, label).clicked() {
                focus = Some(*base);
            }
        }
        for (i, p) in l.capture_points.iter().enumerate() {
            if ui.selectable_label(false, format!("Точка {}", (b'A' + i as u8) as char)).clicked() {
                focus = Some(*p);
            }
        }
        focus
    }

    /// Свойства справа. Возвращает просьбу к лаборатории, если она есть.
    pub fn properties(&mut self, ui: &mut egui::Ui) -> Option<Request> {
        let mut request = None;
        theme::panel(ui, "Инструмент", true, |ui| self.tool_panel(ui));
        theme::panel(ui, "Карта", true, |ui| request = self.map_panel(ui));
        theme::panel(ui, "Атмосфера", true, |ui| self.atmosphere_panel(ui));
        theme::panel(ui, "Проверка", true, |ui| {
            if self.editor.issues.is_empty() {
                ui.label(RichText::new("✔ Всё в порядке: на карте можно играть").color(Color32::from_rgb(120, 200, 110)));
            }
            for issue in &self.editor.issues {
                ui.label(RichText::new(format!("⚠ {issue}")).color(theme::ERROR));
            }
        });
        theme::panel(ui, "Новая карта", false, |ui| self.new_panel(ui));
        theme::panel(ui, "Открыть", true, |ui| self.open_panel(ui));
        if !self.message.0.is_empty() {
            ui.label(RichText::new(&self.message.0).small().color(self.message.1));
        }
        request
    }

    fn tool_panel(&mut self, ui: &mut egui::Ui) {
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
        });
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
                if ui.add_enabled(self.editor.can_undo(), egui::Button::new("↶ Ctrl+Z")).clicked() {
                    self.editor.undo();
                }
                if ui.add_enabled(self.editor.can_redo(), egui::Button::new("↷ Ctrl+Shift+Z")).clicked() {
                    self.editor.redo();
                }
            });
            ui.end_row();
        });
        ui.horizontal(|ui| {
            let save = if self.editor.dirty { "Сохранить *  (Ctrl+S)" } else { "Сохранить  (Ctrl+S)" };
            if ui.button(save).clicked() {
                self.save();
            }
            let playable = self.editor.issues.is_empty();
            if ui.add_enabled(playable, egui::Button::new("▶ Играть на этой карте")).clicked() {
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
        if ui.add_sized([ui.available_width(), 22.0], egui::Button::new("Создать")).clicked() {
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
            if ui.button("Открыть").clicked() {
                let name = self.open_choice.clone();
                self.run(Pending::Open(name));
            }
        });
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
