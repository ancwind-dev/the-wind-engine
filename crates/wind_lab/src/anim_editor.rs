//! Редактор анимаций в студии персонажей: клипы стандарта анимаций из `data/animations`
//! (см. `wind_render::anim`). Точки скелета тянутся мышью прямо во вьюпорте, время — на
//! ленте, клип сохраняется в тот же файл.
//!
//! Внутри правим ключи как кадры «таз + направления отрезков» (`mocap::Clip`): так
//! длины рук и ног не меняются, пока тянешь. При открытии клип переводится в них, при
//! сохранении — обратно в точки; метки, оружие и настройки клипа сохраняются как были.
//! Потянул точку в новом месте ленты — там появился ключ; между ключами — плавно.

use crate::i18n::{f, t};
use crate::icons::{self, Icon};
use crate::theme;
use eframe::egui::{self, Color32, Pos2, Rect, RichText, Sense, Stroke, pos2, vec2};
use std::path::PathBuf;
use std::sync::Arc;
use wind_render::anim::{AnimClip, Body, Hands, Key, Points};
use wind_render::character::{CharModel, CharacterSet, Motion, Skeleton};
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
    /// Ключи клипов в папке.
    names: Vec<String>,
    name: String,
    /// Клип как в файле: метки, оружие, настройки — их редактор не трогает.
    meta: Option<AnimClip>,
    /// Модели — чтобы переводить точки клипа в кадры и обратно (скелет у всех один).
    set: Option<Arc<CharacterSet>>,
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
    pub fn new(dir: PathBuf, set: Option<Arc<CharacterSet>>) -> Self {
        let mut e = AnimEditor {
            on: false,
            dir,
            names: Vec::new(),
            name: String::new(),
            meta: None,
            set,
            clip: None,
            saved: None,
            time: 0.0,
            playing: true,
            soft: 3,
            weapon: true,
            selected: None,
            drag: None,
            undo: Vec::new(),
            new_name: t("новый").into(),
            new_cycle: false,
            message: None,
        };
        e.rescan();
        if let Some(first) = e.names.first().cloned() {
            e.open(&first);
        }
        e
    }

    /// Модели перечитали (сохранили клип) — переводить через новые.
    pub fn set_models(&mut self, set: Option<Arc<CharacterSet>>) {
        self.set = set;
    }

    fn rescan(&mut self) {
        self.names = wind_render::anim::load_dir(&self.dir).0.into_keys().collect();
    }

    fn open(&mut self, name: &str) {
        let path = self.dir.join(AnimClip::file_name(name));
        let result = AnimClip::load(&path).and_then(|a| {
            let m = self.set.as_ref().and_then(|s| s.models.first()).ok_or("нет моделей солдат")?;
            Ok((to_edit(&a, m).ok_or("у модели нет скелета")?, a))
        });
        match result {
            Ok((c, a)) => {
                self.name = name.to_owned();
                self.saved = Some(c.clone());
                self.clip = Some(c);
                self.meta = Some(a);
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
        let Some(m) = self.set.as_ref().and_then(|s| s.models.first()) else { return false };
        let anim = from_edit(clip, m, self.meta.as_ref(), &self.name);
        let result = anim.check().and_then(|()| anim.save(&self.dir));
        match result {
            Ok(path) => {
                self.meta = Some(anim);
                self.saved = self.clip.clone();
                self.message = Some((f("Сохранено: {}", &[&path.display()]), false));
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
        ui.checkbox(&mut self.on, t("Редактировать клип")).on_hover_text(t("Студия показывает клип; точки скелета тянутся левой кнопкой мыши"));
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
            if ui.add_enabled(self.dirty(), egui::Button::new(t("Сохранить"))).clicked() {
                saved = self.save();
            }
            if ui.add_enabled(self.dirty(), egui::Button::new(t("Вернуть"))).on_hover_text(t("Как в файле")).clicked() {
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
            let (icon, play) = if self.playing { (Icon::Pause, t("Пауза")) } else { (Icon::Play, t("Играть")) };
            if icons::button(ui, icon, play).clicked() {
                self.playing = !self.playing;
            }
            ui.label(f("{:.2} с из {:.2}", &[&(self.time.min(1.0) * clip.duration), &clip.duration]));
        });
        theme::props(ui, "anim_props", |ui| {
            if keyed {
                theme::label(ui, t("Ключи"));
                ui.horizontal(|ui| {
                    if ui.button(t("Ключ здесь")).clicked() {
                        self.undo.push(clip.clone());
                        clip.key_at(self.time);
                    }
                    let k = clip.nearest(self.time);
                    if ui.add_enabled(k > 0 && k + 1 < clip.frames.len(), egui::Button::new(t("Убрать ближний"))).clicked() {
                        self.undo.push(clip.clone());
                        clip.remove_key(k);
                    }
                });
                ui.end_row();
                theme::label(ui, t("Длина"));
                ui.add(egui::Slider::new(&mut clip.duration, 0.2..=6.0).suffix(t(" с")));
                ui.end_row();
                theme::label(ui, t("Круг"));
                ui.checkbox(&mut clip.cycle, t("повторяется"));
                ui.end_row();
            } else {
                theme::label(ui, t("Мягкость"));
                ui.add(egui::Slider::new(&mut self.soft, 0..=10).suffix(t(" кадр."))).on_hover_text(t("На сколько соседних кадров расходится правка (слабея)"));
                ui.end_row();
            }
            theme::label(ui, t("Руки"));
            ui.checkbox(&mut self.weapon, t("держат оружие, как в игре"))
                .on_hover_text(t("В шаге и беге руки в игре ставятся к оружию; точки рук на скелете — из клипа (их видно в бросках и своих клипах)"));
            ui.end_row();
        });
        let point = self.selected.map_or("—".to_string(), point_name);
        ui.label(RichText::new(f("Точка: {}. Левая кнопка — выбрать и тянуть; Ctrl+Z — отменить.", &[&point])).small().color(theme::TEXT_DIM));
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
            ui.checkbox(&mut self.new_cycle, t("круг"));
            let free = !self.new_name.trim().is_empty() && !self.names.iter().any(|n| n == self.new_name.trim());
            if ui.add_enabled(free && m.is_some(), egui::Button::new(t("Новый клип"))).on_hover_text(t("Из позы «стоит с оружием»; имя файла — в data/animations")).clicked()
                && let Some(m) = m
                && let Some(sk) = m.skeleton()
            {
                let frame = sk.frame_of(&m.pose(Motion::default()));
                self.name = self.new_name.trim().to_owned();
                self.meta = None;
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

/// Клип стандарта — в кадры для правки (таз + направления), время ключей — доли клипа.
fn to_edit(a: &AnimClip, m: &CharModel) -> Option<Clip> {
    let sk = m.skeleton()?;
    let frames = a.keys.iter().map(|k| m.pose_from_points(&k.points.to_array()).map(|p| sk.frame_of(&p))).collect::<Option<Vec<_>>>()?;
    let length = a.length.max(1e-4);
    let mut times: Vec<f32> = a.keys.iter().map(|k| k.time / length).collect();
    // Правка ждёт ключи в начале и в конце (у круга — в начале).
    let mut frames = frames;
    if times.first().is_some_and(|&t| t > 1e-4) {
        times.insert(0, 0.0);
        frames.insert(0, frames[0].clone());
    }
    if !a.looped && times.last().is_some_and(|&t| t < 1.0 - 1e-4) {
        times.push(1.0);
        frames.push(frames[frames.len() - 1].clone());
    }
    Some(Clip { source: a.source.clone(), cycle: a.looped, duration: a.seconds(), stride: a.stride, peak: 0.0, times, frames })
}

/// Кадры правки — обратно в клип стандарта. Метки и оружие берутся у ключей с тем же
/// временем из `meta` (у новых ключей оружие — между соседними ключами).
fn from_edit(c: &Clip, m: &CharModel, meta: Option<&AnimClip>, key: &str) -> AnimClip {
    let mut out = meta.cloned().unwrap_or_else(|| AnimClip {
        key: key.to_owned(),
        about: String::new(),
        source: c.source.clone(),
        looped: c.cycle,
        length: c.duration,
        speed: 1.0,
        speed_spread: 0.0,
        stride: 0.0,
        body: Body::Full,
        hands: Hands::Clip,
        repeat: None,
        keys: Vec::new(),
    });
    let sk = m.skeleton().expect("скелет");
    out.looped = c.cycle;
    out.length = c.duration * out.speed;
    let old = meta.map(|a| a.keys.clone()).unwrap_or_default();
    let old_len = meta.map_or(out.length, |a| a.length.max(1e-4));
    out.keys = c
        .frames
        .iter()
        .enumerate()
        .map(|(k, f)| {
            let u = c.time_of(k);
            let time = (u * out.length * 1000.0).round() / 1000.0;
            let points = m.points_of(&sk.pose(f)).expect("скелет");
            let same = old.iter().find(|o| (o.time / old_len - u).abs() < 1e-3);
            let weapon = same.and_then(|o| o.weapon).or_else(|| {
                // Новый ключ: оружие — как между соседними старыми ключами.
                let before = old.iter().rev().find(|o| o.time / old_len <= u)?;
                before.weapon
            });
            Key { time, points: Points::from_array(points), weapon, events: same.map(|o| o.events.clone()).unwrap_or_default() }
        })
        .collect();
    out
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
        "hips" => t("таз"),
        "righthip" => t("правое бедро"),
        "lefthip" => t("левое бедро"),
        "rightknee" => t("правое колено"),
        "leftknee" => t("левое колено"),
        "rightfoot" => t("правая стопа"),
        "leftfoot" => t("левая стопа"),
        "midspine" => t("поясница"),
        "neck" => t("шея"),
        "head" => t("голова"),
        "rightshoulder" => t("правое плечо"),
        "leftshoulder" => t("левое плечо"),
        "rightelbow" => t("правый локоть"),
        "leftelbow" => t("левый локоть"),
        "righthand" => t("правая кисть"),
        "lefthand" => t("левая кисть"),
        other => other,
    }
    .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use wind_render::Camera;
    use wind_render::glam::Vec2;

    /// Открыли клип и сохранили без правок — метки, оружие и настройки те же; точки —
    /// те же с точностью до длин отрезков модели (при открытии они выравниваются), а
    /// второй круг «открыл — сохранил» уже ничего не меняет.
    #[test]
    fn clip_survives_open_and_save_unchanged() {
        let data = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../data");
        let set = CharacterSet::load(&data).unwrap();
        let m = &set.models[0];
        let worst = |a: &AnimClip, b: &AnimClip| {
            a.keys.iter().zip(&b.keys).map(|(x, y)| x.points.to_array().iter().zip(&y.points.to_array()).map(|(p, q)| p.distance(*q)).fold(0.0, f32::max)).fold(0.0, f32::max)
        };
        for key in ["reloading", "crouching forwards", "going prone"] {
            let a = AnimClip::load(&data.join("animations").join(AnimClip::file_name(key))).unwrap();
            let once = from_edit(&to_edit(&a, m).unwrap(), m, Some(&a), key);
            let twice = from_edit(&to_edit(&once, m).unwrap(), m, Some(&once), key);
            assert_eq!(once.keys.len(), a.keys.len(), "{key}: ключей");
            assert!(worst(&a, &once) < 0.8, "{key}: точка ушла на {:.2} вокселя", worst(&a, &once));
            assert!(worst(&once, &twice) < 0.05, "{key}: второй круг сдвинул точку на {:.2}", worst(&once, &twice));
            for (k0, k1) in a.keys.iter().zip(&once.keys) {
                assert!((k0.time - k1.time).abs() < 2e-3, "{key}: время {} → {}", k0.time, k1.time);
                assert_eq!(k0.events, k1.events, "{key}: метки");
                assert_eq!(k0.weapon, k1.weapon, "{key}: оружие");
            }
            assert_eq!((once.looped, once.body, once.hands, once.repeat), (a.looped, a.body, a.hands, a.repeat));
            assert!((once.length - a.length).abs() < 1e-4);
        }
    }

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
        let clip = to_edit(&AnimClip::load(&data.join("animations/running.ron")).unwrap(), m).unwrap();
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

