//! Редактор анимаций в студии персонажей: клипы из `data/animations` (шаг, бег, бросок
//! из записей движений и свои). Точки скелета тянутся мышью прямо во вьюпорте, время —
//! на ленте, клип сохраняется в тот же файл.
//!
//! У клипа из записи кадр — каждые 1/40 клипа: правка мягко расходится на соседние кадры
//! («Мягкость»). Свой клип — ключевые кадры: потянул точку в новом месте ленты — там
//! появился ключ; между ключами — плавно.

use crate::icons::{self, Icon};
use crate::theme;
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, pos2, vec2};
use std::path::PathBuf;
use wind_render::character::{CharModel, Motion, Skeleton};
use wind_render::glam::{Mat4, Vec3, Vec4Swizzles};
use wind_render::mocap::{Clip, EDGES};
use wind_render::rwr::VOXEL;

/// Тянем точку: какую, с какого клипа начали (для «мягкости» — без накопления) и ключ.
struct Drag {
    point: usize,
    base: Clip,
    key: usize,
    /// Где была точка, когда её взяли (мир, м): тянем в плоскости экрана через неё.
    anchor: Vec3,
}

pub struct AnimEditor {
    /// Редактор включён: студия показывает клип, а не движение.
    pub on: bool,
    dir: PathBuf,
    names: Vec<String>,
    name: String,
    clip: Option<Clip>,
    /// Клип как в файле — «вернуть» и признак несохранённых правок.
    saved: Option<Clip>,
    /// Доля клипа, которую показываем.
    pub time: f32,
    pub playing: bool,
    /// На сколько соседних кадров расходится правка у клипа из записи.
    soft: usize,
    /// Руки — к оружию, как в игре (для шага и бега руки из записи в игре не видны).
    pub weapon: bool,
    selected: Option<usize>,
    drag: Option<Drag>,
    undo: Vec<Clip>,
    new_name: String,
    new_cycle: bool,
    message: Option<(String, bool)>,
}

impl AnimEditor {
    pub fn new(dir: PathBuf) -> Self {
        let mut e = AnimEditor {
            on: false,
            dir,
            names: Vec::new(),
            name: String::new(),
            clip: None,
            saved: None,
            time: 0.0,
            playing: true,
            soft: 3,
            weapon: true,
            selected: None,
            drag: None,
            undo: Vec::new(),
            new_name: "новый".into(),
            new_cycle: false,
            message: None,
        };
        e.rescan();
        if let Some(first) = e.names.first().cloned() {
            e.open(&first);
        }
        e
    }

    fn rescan(&mut self) {
        self.names = std::fs::read_dir(&self.dir)
            .map(|d| {
                let mut v: Vec<String> = d
                    .filter_map(|e| e.ok())
                    .filter_map(|e| e.file_name().to_str()?.strip_suffix(".ron").map(str::to_owned))
                    .collect();
                v.sort();
                v
            })
            .unwrap_or_default();
    }

    fn open(&mut self, name: &str) {
        match Clip::load(&self.dir.join(format!("{name}.ron"))) {
            Ok(c) => {
                self.name = name.to_owned();
                self.saved = Some(c.clone());
                self.clip = Some(c);
                self.undo.clear();
                self.selected = None;
                self.message = None;
            }
            Err(e) => self.message = Some((e, true)),
        }
    }

    fn dirty(&self) -> bool {
        self.clip != self.saved
    }

    /// Сохранить клип. `true` — сохранён: студии пора перечитать модели.
    fn save(&mut self) -> bool {
        let Some(clip) = &self.clip else { return false };
        let path = self.dir.join(format!("{}.ron", self.name));
        let header = format!(
            "// Клип «{}». Источник: {}.\n// Правится в студии персонажей (лаборатория → «Редактор анимаций»); оси: x — вперёд, y — вверх, z — вправо.\n",
            self.name, clip.source
        );
        let result = clip.to_ron().and_then(|body| std::fs::write(&path, header + &body).map_err(|e| format!("{}: {e}", path.display())));
        match result {
            Ok(()) => {
                self.saved = self.clip.clone();
                self.message = Some((format!("Сохранено: {}", path.display()), false));
                self.rescan();
                true
            }
            Err(e) => {
                self.message = Some((e, true));
                false
            }
        }
    }

    /// Время идёт, пока играем (`dt` — уже с замедлением студии).
    pub fn advance(&mut self, dt: f32) {
        let Some(c) = &self.clip else { return };
        if self.playing && self.drag.is_none() {
            self.time += dt / c.duration.max(0.05);
            // Действие повторяем с паузой в 0,3 клипа — видно начало и конец.
            self.time = if c.cycle { self.time.rem_euclid(1.0) } else if self.time > 1.3 { 0.0 } else { self.time };
        }
    }

    /// Поза модели на текущем времени (воксели, оси модели).
    pub fn pose(&self, m: &CharModel) -> Option<Vec<Vec3>> {
        let sk = m.skeleton()?;
        let pose = sk.pose(&self.clip.as_ref()?.sample(self.time.min(1.0)));
        Some(if self.weapon { m.with_weapon(pose) } else { pose })
    }

    /// Правая панель. `m` — модель студии (для нового клипа). Возвращает `true`, если
    /// клип сохранён.
    pub fn panel(&mut self, ui: &mut egui::Ui, m: Option<&CharModel>) -> bool {
        let mut saved = false;
        ui.checkbox(&mut self.on, "Редактировать клип").on_hover_text("Студия показывает клип; точки скелета тянутся левой кнопкой мыши");
        if !self.on {
            return false;
        }
        ui.horizontal(|ui| {
            let label = if self.dirty() { format!("{} •", self.name) } else { self.name.clone() };
            egui::ComboBox::from_id_salt("anim_clip").selected_text(label).show_ui(ui, |ui| {
                for n in self.names.clone() {
                    if ui.selectable_label(n == self.name, &n).clicked() {
                        self.open(&n);
                    }
                }
            });
            if ui.add_enabled(self.dirty(), egui::Button::new("Сохранить")).clicked() {
                saved = self.save();
            }
            if ui.add_enabled(self.dirty(), egui::Button::new("Вернуть")).on_hover_text("Как в файле").clicked() {
                self.clip = self.saved.clone();
                self.undo.clear();
            }
        });
        let Some(clip) = self.clip.as_mut() else {
            self.new_clip_ui(ui, m);
            return saved;
        };
        let keyed = clip.is_keyed();
        ui.label(RichText::new(&clip.source).small().color(theme::TEXT_DIM));

        // Лента времени: риски — кадры (у своего клипа — ключи), линия — сейчас.
        let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click_and_drag());
        let painter = ui.painter_at(r);
        painter.rect_filled(r, 4, Color32::from_gray(30));
        for k in 0..clip.frames.len() {
            let x = r.left() + clip.time_of(k) * r.width();
            let (h, c) = if keyed { (r.height() * 0.8, Color32::from_rgb(240, 200, 80)) } else { (r.height() * 0.3, Color32::from_gray(90)) };
            painter.line_segment([pos2(x, r.center().y - h / 2.0), pos2(x, r.center().y + h / 2.0)], Stroke::new(if keyed { 3.0 } else { 1.0 }, c));
        }
        let x = r.left() + self.time.min(1.0) * r.width();
        painter.line_segment([pos2(x, r.top()), pos2(x, r.bottom())], Stroke::new(2.0, Color32::WHITE));
        if let Some(p) = resp.interact_pointer_pos() {
            self.time = ((p.x - r.left()) / r.width()).clamp(0.0, 1.0);
            self.playing = false;
        }
        ui.horizontal(|ui| {
            let (icon, play) = if self.playing { (Icon::Pause, "Пауза") } else { (Icon::Play, "Играть") };
            if icons::button(ui, icon, play).clicked() {
                self.playing = !self.playing;
            }
            ui.label(format!("{:.2} с из {:.2}", self.time.min(1.0) * clip.duration, clip.duration));
        });
        theme::props(ui, "anim_props", |ui| {
            if keyed {
                theme::label(ui, "Ключи");
                ui.horizontal(|ui| {
                    if ui.button("Ключ здесь").clicked() {
                        self.undo.push(clip.clone());
                        clip.key_at(self.time);
                    }
                    let k = clip.nearest(self.time);
                    if ui.add_enabled(k > 0 && k + 1 < clip.frames.len(), egui::Button::new("Убрать ближний")).clicked() {
                        self.undo.push(clip.clone());
                        clip.remove_key(k);
                    }
                });
                ui.end_row();
                theme::label(ui, "Длина");
                ui.add(egui::Slider::new(&mut clip.duration, 0.2..=6.0).suffix(" с"));
                ui.end_row();
                theme::label(ui, "Круг");
                ui.checkbox(&mut clip.cycle, "повторяется");
                ui.end_row();
            } else {
                theme::label(ui, "Мягкость");
                ui.add(egui::Slider::new(&mut self.soft, 0..=10).suffix(" кадр.")).on_hover_text("На сколько соседних кадров расходится правка (слабея)");
                ui.end_row();
            }
            theme::label(ui, "Руки");
            ui.checkbox(&mut self.weapon, "держат оружие, как в игре")
                .on_hover_text("В шаге и беге руки в игре ставятся к оружию; точки рук на скелете — из клипа (их видно в бросках и своих клипах)");
            ui.end_row();
        });
        let point = self.selected.map_or("—".to_string(), point_name);
        ui.label(RichText::new(format!("Точка: {point}. Левая кнопка — выбрать и тянуть; Ctrl+Z — отменить.")).small().color(theme::TEXT_DIM));
        if ui.input(|i| i.modifiers.command && i.key_pressed(egui::Key::Z))
            && let Some(prev) = self.undo.pop()
        {
            *clip = prev;
        }
        ui.separator();
        self.new_clip_ui(ui, m);
        if let Some((text, error)) = &self.message {
            ui.label(RichText::new(text).small().color(if *error { theme::ERROR } else { theme::TEXT_DIM }));
        }
        saved
    }

    /// Новый клип из позы «стоит с оружием»: два одинаковых ключа, дальше — править.
    fn new_clip_ui(&mut self, ui: &mut egui::Ui, m: Option<&CharModel>) {
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.new_name).desired_width(110.0));
            ui.checkbox(&mut self.new_cycle, "круг");
            let free = !self.new_name.trim().is_empty() && !self.names.iter().any(|n| n == self.new_name.trim());
            if ui.add_enabled(free && m.is_some(), egui::Button::new("Новый клип")).on_hover_text("Из позы «стоит с оружием»; имя файла — в data/animations").clicked()
                && let Some(m) = m
                && let Some(sk) = m.skeleton()
            {
                let frame = sk.frame_of(&m.pose(Motion::default()));
                self.name = self.new_name.trim().to_owned();
                self.clip = Some(Clip::keyed("свой клип, сделан в студии персонажей", frame, 1.0, self.new_cycle));
                self.saved = None;
                self.undo.clear();
                self.time = 0.0;
                self.weapon = false;
            }
        });
    }

    /// Скелет поверх солдата и правка мышью. `root` — где солдат (модель → мир, м).
    pub fn interact(&mut self, ui: &egui::Ui, rect: Rect, view_proj: Mat4, eye: Vec3, root: Mat4, m: &CharModel) {
        let (Some(sk), Some(clip)) = (m.skeleton(), self.clip.as_mut()) else { return };
        let frame = clip.sample(self.time.min(1.0));
        let world: Vec<Vec3> = sk.points(&frame).iter().map(|&p| root.transform_point3(p * VOXEL)).collect();
        let to_screen = |p: Vec3| {
            let c = view_proj * p.extend(1.0);
            let n = c.xyz() / c.w;
            pos2(rect.left() + (n.x * 0.5 + 0.5) * rect.width(), rect.top() + (0.5 - n.y * 0.5) * rect.height())
        };
        let screen: Vec<Pos2> = world.iter().map(|&p| to_screen(p)).collect();
        let (hover, pressed, down) = ui.input(|i| (i.pointer.hover_pos(), i.pointer.primary_pressed(), i.pointer.primary_down()));
        let inside = hover.filter(|p| rect.contains(*p));
        let near = inside.and_then(|p| (0..screen.len()).filter(|&i| screen[i].distance(p) < 12.0).min_by(|&a, &b| screen[a].distance(p).total_cmp(&screen[b].distance(p))));

        // Взяли точку — запоминаем клип (для отмены) и ключ, в который пишем.
        if pressed && let Some(i) = near {
            self.undo.push(clip.clone());
            let key = clip.key_at(self.time.min(1.0));
            self.time = clip.time_of(key);
            self.selected = Some(i);
            self.playing = false;
            self.drag = Some(Drag { point: i, base: clip.clone(), key, anchor: world[i] });
        }
        if let Some(d) = &self.drag {
            match inside.filter(|_| down) {
                Some(p) => {
                    if let Some(hit) = screen_to_plane(view_proj, rect, p, d.anchor, eye) {
                        let target = root.inverse().transform_point3(hit) / VOXEL;
                        let edited = sk.drag(&d.base.frames[d.key], d.point, target);
                        let mut c = d.base.clone();
                        c.apply(d.key, &edited, self.soft);
                        *clip = c;
                    }
                }
                None => self.drag = None,
            }
        }

        // Скелет: отрезки, точки; выбранная — жёлтая, под курсором — крупнее.
        let painter = ui.painter_at(rect);
        let line = Color32::from_rgb(80, 220, 255);
        for (e, &(from, _)) in EDGES.iter().enumerate() {
            painter.line_segment([screen[Skeleton::point_of(from)], screen[1 + e]], Stroke::new(1.5, line));
        }
        for (i, &p) in screen.iter().enumerate() {
            let color = if Some(i) == self.selected { Color32::from_rgb(255, 210, 60) } else { line };
            painter.circle_filled(p, if Some(i) == near { 6.0 } else { 4.0 }, color);
        }
    }
}

/// Точка экрана `p` — в мире: луч из камеры через неё до плоскости, обращённой к камере
/// и проходящей через `anchor` (тянем точку «по экрану», не меняя глубины).
fn screen_to_plane(view_proj: Mat4, rect: Rect, p: Pos2, anchor: Vec3, eye: Vec3) -> Option<Vec3> {
    let ndc = vec2((p.x - rect.left()) / rect.width() * 2.0 - 1.0, 1.0 - (p.y - rect.top()) / rect.height() * 2.0);
    let inv = view_proj.inverse();
    let a = inv.project_point3(Vec3::new(ndc.x, ndc.y, 0.0));
    let b = inv.project_point3(Vec3::new(ndc.x, ndc.y, 1.0));
    let n = (eye - anchor).normalize_or(Vec3::Y);
    let dir = b - a;
    (dir.dot(n).abs() > 1e-6).then(|| a + dir * ((anchor - a).dot(n) / dir.dot(n)))
}

/// Название точки по номеру (как в `Skeleton::points`).
fn point_name(i: usize) -> String {
    let name = if i == 0 { "hips" } else { EDGES[i - 1].1 };
    match name {
        "hips" => "таз",
        "righthip" => "правое бедро",
        "lefthip" => "левое бедро",
        "rightknee" => "правое колено",
        "leftknee" => "левое колено",
        "rightfoot" => "правая стопа",
        "leftfoot" => "левая стопа",
        "midspine" => "поясница",
        "neck" => "шея",
        "head" => "голова",
        "rightshoulder" => "правое плечо",
        "leftshoulder" => "левое плечо",
        "rightelbow" => "правый локоть",
        "leftelbow" => "левый локоть",
        "righthand" => "правая кисть",
        "lefthand" => "левая кисть",
        other => other,
    }
    .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wind_render::Camera;
    use wind_render::glam::Vec2;

    #[test]
    fn screen_point_maps_back_to_the_world() {
        let mut cam = Camera { target: Vec2::new(32.0, 32.0), distance: 3.5, pitch: 0.35, ..Default::default() };
        cam.target_height = 0.9;
        let rect = Rect::from_min_size(pos2(100.0, 50.0), vec2(1130.0, 890.0));
        let view_proj = cam.view_proj(rect.width() / rect.height());
        let eye = cam.eye();
        let anchor = Vec3::new(32.2, 1.3, 31.9);
        let to_screen = |w: Vec3| {
            let c = view_proj * w.extend(1.0);
            let n = c.xyz() / c.w;
            pos2(rect.left() + (n.x * 0.5 + 0.5) * rect.width(), rect.top() + (0.5 - n.y * 0.5) * rect.height())
        };
        // Сама точка возвращается в себя.
        let back = screen_to_plane(view_proj, rect, to_screen(anchor), anchor, eye).unwrap();
        assert!(back.distance(anchor) < 1e-3, "{back:?}");
        // Курсор правее и выше — точка уходит вправо и вверх по экрану, глубина та же.
        let moved = screen_to_plane(view_proj, rect, to_screen(anchor) + vec2(40.0, -30.0), anchor, eye).unwrap();
        let s = to_screen(moved);
        assert!((s - (to_screen(anchor) + vec2(40.0, -30.0))).length() < 0.5, "{s:?}");
        assert!(((moved - anchor).dot(eye - anchor)).abs() < 1e-3, "глубина не меняется");
    }

    #[test]
    fn dragging_a_hand_in_the_world_moves_it_there() {
        let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let set = wind_render::character::CharacterSet::load(&data).unwrap();
        let m = &set.models[0];
        let sk = m.skeleton().unwrap();
        let clip = Clip::load(&data.join("animations/run.ron")).unwrap();
        // Солдат стоит в (32, 32) и смотрит под углом 1 рад — как в студии.
        let root = Mat4::from_translation(Vec3::new(32.0, 0.0, 32.0)) * Mat4::from_rotation_y(-1.0);
        let frame = clip.sample(0.3);
        let hand = Skeleton::point_of("righthand");
        let world = root.transform_point3(sk.points(&frame)[hand] * VOXEL);
        // Тянем кисть на 8 см вверх и чуть в сторону — в мире.
        let hit = world + Vec3::new(0.05, 0.08, -0.03);
        let target = root.inverse().transform_point3(hit) / VOXEL;
        let edited = sk.drag(&frame, hand, target);
        let now = root.transform_point3(sk.points(&edited)[hand] * VOXEL);
        assert!(now.distance(hit) < 0.005, "кисть в {now:?}, а тянули в {hit:?}");
    }
}

