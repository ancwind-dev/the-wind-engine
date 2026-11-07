//! Студия персонажей: один солдат вблизи. Модель, оружие, стойка (стоит, присел, лежит),
//! шаг и бег, падение, скелет поверх модели и ширина плавного сгиба у суставов — чтобы
//! подбирать вид солдата, не запуская бой. Здесь же — редактор анимаций (см.
//! `anim_editor`) и, ВРЕМЕННО, справка RWR: их анимации на соседнем солдате для сверки.

use crate::i18n::{f, t};
use crate::anim_editor::AnimEditor;
use crate::icons::{self, Icon};
use crate::theme;
use crate::viewport::{Scene, SceneSoldier, Viewport};
use eframe::egui::{self, Color32, RichText, Stroke, pos2};
use std::sync::Arc;
use std::time::Instant;
use wind_render::character::CharacterSet;
use wind_render::glam::{Mat4, Vec3, Vec4Swizzles};
use wind_render::{Action, JOINT_BLEND, Look, Motion, THROW_TIME};
use wind_render::rwr_anim::RwrAnim;
use wind_sim::{Stance, Vec2};
use wind_sim::atmosphere::Atmosphere;
use wind_sim::map::Map;

/// Площадка: ровная земля, солдат в центре.
const GROUND: u32 = 64;
/// Камера студии — сбоку и чуть сверху, как в редакторе моделей RWR.
const CAMERA_PITCH: f32 = 0.35;

pub struct Studio {
    pub view: Viewport,
    characters: Option<Arc<CharacterSet>>,
    map: Map,
    atmosphere: Atmosphere,
    model: usize,
    weapon: Option<usize>,
    team: u8,
    /// Скорость шага, м/с: 0 — стоит.
    speed: f32,
    /// Фаза шага и возраст тела — двигаются сами, пока не на паузе.
    phase: f32,
    /// Куда идёт относительно взгляда, градусы: 0 — вперёд, 90 — вправо, 180 — назад.
    heading: f32,
    /// Целится: на бегу оружие вперёд, а не поперёк груди.
    aiming: bool,
    dead: Option<f32>,
    /// Чем убит: пулей спереди или взрывом спереди.
    blast: bool,
    /// Действие руками — повторяется по кругу — и его часы.
    act: Act,
    act_time: f32,
    paused: bool,
    /// Замедление: 1 — как в игре.
    tempo: f32,
    facing: f32,
    blend: f32,
    skeleton: bool,
    last: Instant,
    /// Редактор анимаций: клипы `data/animations`.
    editor: AnimEditor,
    /// Модели перечитаны (клип сохранён) — лаборатории обновить остальные виды.
    reloaded: Option<Arc<CharacterSet>>,
    /// Папка данных проекта: модели и клипы.
    data: std::path::PathBuf,
    /// Стойка, к которой идёт солдат, и как далеко он присел / лёг (как в игре).
    stance: Stance,
    crouch: f32,
    prone: f32,
    rising: bool,
    /// Часы студии (для клипов на месте — дыхание).
    clock: f32,
    /// ВРЕМЕННО: справка RWR — анимации из установленной игры, рядом с нашим солдатом.
    rwr: Option<Result<Vec<RwrAnim>, String>>,
    rwr_on: bool,
    rwr_filter: String,
    rwr_pick: usize,
    rwr_time: f32,
    rwr_playing: bool,
}

impl Studio {
    pub fn new(characters: Option<Arc<CharacterSet>>, has_3d: bool, data: std::path::PathBuf) -> Self {
        let mut view = Viewport::new(&data);
        view.has_3d = has_3d;
        view.set_characters(characters.clone());
        let view_characters = characters.clone();
        let center = Vec2::splat(GROUND as f32 * 0.5);
        view.aim_height = 0.9;
        view.look_at_3d(center, 3.5, CAMERA_PITCH);
        Studio {
            view,
            characters,
            map: {
                // Площадка — утоптанная земля без травы: трава закрывала бы стопы и колени.
                let mut map = Map::new(GROUND, GROUND);
                map.ground.fill(2);
                map
            },
            atmosphere: Atmosphere::default(),
            model: 0,
            weapon: Some(0),
            team: 0,
            speed: 2.2,
            phase: 0.0,
            heading: 0.0,
            aiming: false,
            dead: None,
            blast: false,
            act: Act::None,
            act_time: 0.0,
            paused: false,
            tempo: 1.0,
            facing: -0.6,
            blend: JOINT_BLEND,
            skeleton: false,
            last: Instant::now(),
            editor: AnimEditor::new(data.join("animations"), view_characters),
            data,
            reloaded: None,
            stance: Stance::Stand,
            crouch: 0.0,
            prone: 0.0,
            rising: false,
            clock: 0.0,
            rwr: None,
            rwr_on: false,
            rwr_filter: String::new(),
            rwr_pick: 0,
            rwr_time: 0.0,
            rwr_playing: true,
        }
    }

    /// Модели перечитаны после сохранения клипа — один раз.
    pub fn take_reloaded(&mut self) -> Option<Arc<CharacterSet>> {
        self.reloaded.take()
    }

    /// Перечитать модели с клипами — после сохранения клипа.
    fn reload_characters(&mut self) {
        match CharacterSet::load(&self.data) {
            Ok(set) => {
                let set = Arc::new(set);
                self.characters = Some(set.clone());
                self.view.set_characters(Some(set.clone()));
                self.editor.set_models(Some(set.clone()));
                self.reloaded = Some(set);
            }
            Err(e) => eprintln!("модели не перечитались: {e}"),
        }
    }

    fn center(&self) -> Vec2 {
        Vec2::splat(GROUND as f32 * 0.5)
    }

    /// Слева — модели и оружие.
    pub fn outliner(&mut self, ui: &mut egui::Ui) {
        let Some(set) = self.characters.clone() else {
            ui.label(RichText::new(t("Нет своих моделей: не найден data/factions.ron")).color(theme::ERROR));
            return;
        };
        ui.label(RichText::new(t("Модели")).strong());
        // Имя вида «soldiers/usmc/rifleman»: группируем по фракции, показываем последнее слово.
        let mut faction = "";
        for (i, m) in set.models.iter().enumerate() {
            let mut parts = m.name.rsplit('/');
            let short = parts.next().unwrap_or(&m.name);
            let group = parts.next().unwrap_or("");
            if group != faction {
                faction = group;
                ui.label(RichText::new(group).small().color(theme::TEXT_DIM));
            }
            if icons::selectable(ui, self.model == i, Icon::Soldier, short, None).clicked() {
                self.model = i;
            }
        }
        ui.add_space(8.0);
        ui.label(RichText::new(t("Оружие")).strong());
        if icons::selectable(ui, self.weapon.is_none(), Icon::Soldier, t("без оружия"), None).clicked() {
            self.weapon = None;
        }
        for (i, w) in set.weapons.iter().enumerate() {
            let short = w.name.rsplit('/').next().unwrap_or(&w.name);
            if icons::selectable(ui, self.weapon == Some(i), Icon::Weapon, short, None).clicked() {
                self.weapon = Some(i);
            }
        }
    }

    /// Справа — анимация, сгиб, вид.
    pub fn properties(&mut self, ui: &mut egui::Ui) {
        theme::panel(ui, Icon::Run, t("Анимация"), true, |ui| {
            // Стойка, как в игре: присесть и встать — плавно, лечь и встать — клипом.
            ui.horizontal(|ui| {
                for (name, stance) in [(t("Стоит"), Stance::Stand), (t("Присел"), Stance::Crouch), (t("Лежит"), Stance::Prone)] {
                    if ui.selectable_label(self.stance == stance, name).clicked() {
                        self.stance = stance;
                        // Скорость — под стойку: приседом и ползком медленнее.
                        self.speed = match stance {
                            Stance::Stand => self.speed.min(6.0),
                            Stance::Crouch => self.speed.min(wind_render::character::CROUCH_SPEED),
                            Stance::Prone => self.speed.min(wind_render::character::PRONE_SPEED),
                        };
                        self.dead = None;
                    }
                }
            });
            ui.horizontal(|ui| {
                for (name, speed) in [(t("На месте"), 0.0), (t("Шаг"), 2.2), (t("Бег"), 6.0)] {
                    if ui.selectable_label(self.dead.is_none() && self.speed == speed, name).clicked() {
                        self.speed = speed;
                        self.dead = None;
                    }
                }
                if ui.selectable_label(self.dead.is_some(), t("Падает")).clicked() {
                    self.dead = Some(0.0);
                }
            });
            ui.horizontal(|ui| {
                for (name, act) in [(t("Без действия"), Act::None), (t("Перезарядка"), Act::Reload), (t("Бросок гранаты"), Act::Throw), (t("Стрельба"), Act::Fire)] {
                    if ui.selectable_label(self.dead.is_none() && self.act == act, name).clicked() {
                        self.act = act;
                        self.act_time = 0.0;
                        self.dead = None;
                    }
                }
            });
            theme::props(ui, "studio_anim", |ui| {
                theme::label(ui, t("Скорость"));
                ui.add(egui::Slider::new(&mut self.speed, 0.0..=8.0).suffix(t(" м/с")));
                ui.end_row();
                theme::label(ui, t("Замедление"));
                ui.add(egui::Slider::new(&mut self.tempo, 0.05..=1.0).logarithmic(true).suffix("×"));
                ui.end_row();
                theme::label(ui, t("Фаза шага"));
                ui.add(egui::Slider::new(&mut self.phase, 0.0..=std::f32::consts::TAU).max_decimals(2));
                ui.end_row();
                theme::label(ui, t("Направление"));
                ui.add(egui::Slider::new(&mut self.heading, -180.0..=180.0).suffix("°").max_decimals(0))
                    .on_hover_text(t("Куда идёт относительно взгляда: 0 — вперёд, 90 — вправо, 180 — назад"));
                ui.end_row();
                theme::label(ui, t("Оружие"));
                ui.checkbox(&mut self.aiming, t("целится")).on_hover_text(t("Не целясь, на бегу солдат несёт оружие поперёк груди, как в RWR"));
                ui.end_row();
                theme::label(ui, t("Чем убит"));
                ui.horizontal(|ui| {
                    // Выбрали — сразу падает заново.
                    if ui.selectable_value(&mut self.blast, false, t("пулей")).clicked() | ui.selectable_value(&mut self.blast, true, t("взрывом")).clicked() {
                        self.dead = Some(0.0);
                    }
                });
                ui.end_row();
            });
            let (icon, play) = if self.paused { (Icon::Play, t("Дальше")) } else { (Icon::Pause, t("Пауза")) };
            if icons::button(ui, icon, play).clicked() {
                self.paused = !self.paused;
            }
        });
        let model = self.characters.clone();
        let m = model.as_ref().and_then(|c| c.models.get(self.model));
        let mut saved = false;
        theme::panel(ui, Icon::Run, t("Редактор анимаций"), true, |ui| {
            saved = self.editor.panel(ui, m);
        });
        if saved {
            self.reload_characters();
        }
        theme::panel(ui, Icon::Info, t("Справка RWR (временно)"), false, |ui| self.rwr_panel(ui));
        theme::panel(ui, Icon::Skeleton, t("Сгиб у суставов"), true, |ui| {
            theme::props(ui, "studio_blend", |ui| {
                theme::label(ui, t("Ширина"));
                ui.add(egui::Slider::new(&mut self.blend, 0.0..=12.0).suffix(t(" вокс.")));
                ui.end_row();
            });
            ui.horizontal(|ui| {
                if ui.button(t("Жёстко (0)")).clicked() {
                    self.blend = 0.0;
                }
                if ui.button(f("Как в игре ({})", &[&JOINT_BLEND])).clicked() {
                    self.blend = JOINT_BLEND;
                }
            });
            ui.label(
                RichText::new(t("Воксель у сустава смешивает положение своей палки и соседней. В игре сгиб — как по умолчанию."))
                    .small()
                    .color(theme::TEXT_DIM),
            );
        });
        theme::panel(ui, Icon::Camera, t("Вид"), true, |ui| {
            theme::props(ui, "studio_view", |ui| {
                theme::label(ui, t("Скелет"));
                ui.checkbox(&mut self.skeleton, t("поверх модели"));
                ui.end_row();
                theme::label(ui, t("Поворот"));
                ui.drag_angle(&mut self.facing);
                ui.end_row();
                theme::label(ui, t("Команда"));
                ui.horizontal(|ui| {
                    for t in 0..2u8 {
                        ui.selectable_value(&mut self.team, t, RichText::new(theme::team_name(t as usize)).color(theme::TEAM[t as usize]));
                    }
                });
                ui.end_row();
            });
            if icons::button(ui, Icon::Camera, t("Камеру к солдату")).clicked() {
                self.view.look_at_3d(self.center(), 3.5, CAMERA_PITCH);
            }
        });
        if let Some(set) = &self.characters
            && let Some(m) = set.models.get(self.model)
        {
            theme::panel(ui, Icon::Info, t("Сведения"), false, |ui| {
                let mesh = &set.meshes[m.mesh];
                theme::props(ui, "studio_info", |ui| {
                    theme::label(ui, t("Палок"));
                    ui.label(m.sticks.len().to_string());
                    ui.end_row();
                    theme::label(ui, t("Точек"));
                    ui.label(format!("{} / {} / {}", mesh.lods[0].points.len() / 4, mesh.lods[1].points.len() / 4, mesh.lods[2].points.len() / 4));
                    ui.end_row();
                    theme::label(ui, t("Граней тени"));
                    ui.label((mesh.indices.len() / 6).to_string());
                    ui.end_row();
                });
            });
        }
    }

    /// Стойка студии сразу (ключ `--stance`): без перехода.
    pub fn set_stance(&mut self, stance: Stance, speed: f32) {
        self.stance = stance;
        self.crouch = if stance == Stance::Crouch { 1.0 } else { 0.0 };
        self.prone = if stance == Stance::Prone { 1.0 } else { 0.0 };
        self.speed = speed;
    }

    /// Действие руками сразу (ключ `--act fire|reload|throw`, для снимков).
    pub fn set_act(&mut self, name: &str) {
        self.act = match name {
            "fire" => Act::Fire,
            "reload" => Act::Reload,
            "throw" => Act::Throw,
            _ => Act::None,
        };
        self.act_time = 0.0;
    }

    /// Солдат RWR рядом с анимацией `name` (ключ `--rwr`, для снимков).
    pub fn show_rwr(&mut self, name: &str) {
        self.rwr_on = true;
        self.frame_both();
        let all = self.rwr.get_or_insert_with(wind_render::rwr_anim::load_install);
        if let Ok(list) = all
            && let Some(a) = list.iter().find(|a| a.name == name)
        {
            self.rwr_pick = a.index;
        }
    }

    /// Где стоит солдат RWR — справа от нашего.
    fn rwr_spot(&self) -> Vec2 {
        self.center() + Vec2::from_angle(self.facing + std::f32::consts::FRAC_PI_2) * 1.3
    }

    /// Камера — на обоих солдат.
    fn frame_both(&mut self) {
        let mid = (self.center() + self.rwr_spot()) * 0.5;
        self.view.look_at_3d(mid, 4.6, CAMERA_PITCH);
    }

    /// Выбранная анимация RWR.
    fn rwr_selected(&self) -> Option<&RwrAnim> {
        self.rwr.as_ref()?.as_ref().ok()?.get(self.rwr_pick)
    }

    /// ВРЕМЕННО: анимации RWR из установленной игры — для сверки, в проект не попадают.
    fn rwr_panel(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new(t("Анимации из установленной RWR — только посмотреть и сравнить: в проект они не копируются.")).small().color(theme::TEXT_DIM));
        if ui.checkbox(&mut self.rwr_on, t("Солдат RWR рядом")).changed() {
            if self.rwr_on {
                self.frame_both();
            } else {
                self.view.look_at_3d(self.center(), 3.5, CAMERA_PITCH);
            }
        }
        if !self.rwr_on {
            return;
        }
        let all = self.rwr.get_or_insert_with(wind_render::rwr_anim::load_install);
        let list = match all {
            Ok(list) => list.clone(),
            Err(e) => {
                ui.label(RichText::new(e.as_str()).color(theme::ERROR));
                return;
            }
        };
        ui.add(egui::TextEdit::singleline(&mut self.rwr_filter).hint_text(t("Поиск: running, crouch, prone…")));
        let filter = self.rwr_filter.to_lowercase();
        egui::ScrollArea::vertical().id_salt("rwr_list").max_height(160.0).show(ui, |ui| {
            for a in list.iter().filter(|a| filter.is_empty() || a.name.to_lowercase().contains(&filter)) {
                let label = format!("#{} {}", a.index, a.name);
                if ui.selectable_label(self.rwr_pick == a.index, label).clicked() {
                    self.rwr_pick = a.index;
                    self.rwr_time = 0.0;
                }
            }
        });
        if let Some(a) = list.get(self.rwr_pick) {
            let end = a.end.max(a.frames.last().map_or(0.0, |f| f.time)).max(1e-3);
            if !a.looped && self.rwr_time > end + 0.4 {
                self.rwr_time = 0.0;
            }
            ui.horizontal(|ui| {
                let (icon, play) = if self.rwr_playing { (Icon::Pause, t("Пауза")) } else { (Icon::Play, t("Играть")) };
                if icons::button(ui, icon, play).clicked() {
                    self.rwr_playing = !self.rwr_playing;
                }
                let mut shown = if a.looped { self.rwr_time.rem_euclid(end) } else { self.rwr_time.min(end) };
                if ui.add(egui::Slider::new(&mut shown, 0.0..=end).max_decimals(2)).changed() {
                    self.rwr_time = shown;
                    self.rwr_playing = false;
                }
            });
            ui.label(RichText::new(a.report()).small().monospace().color(theme::TEXT_DIM));
        }
    }

    /// Действие руками сейчас: повторяется с паузой между повторами. Перезарядка —
    /// по выбранному оружию (как и сколько секунд).
    fn action(&self) -> Option<Action> {
        match self.act {
            Act::None | Act::Fire => None,
            Act::Reload => {
                let w = self.characters.as_ref()?.weapons.get(self.weapon?)?;
                let t = (self.act_time % (w.reload_time + 0.6)) / w.reload_time.max(0.1);
                Some(Action::Reload { kind: w.reload, t: t.min(1.0), rounds: w.rounds })
            }
            Act::Throw => Some(Action::Throw { t: (self.act_time % (THROW_TIME + 0.8)).min(THROW_TIME) }),
        }
    }

    /// Вьюпорт: солдат и, если нужно, его скелет.
    pub fn viewport(&mut self, ui: &mut egui::Ui) {
        let now = Instant::now();
        let dt = now.duration_since(self.last).as_secs_f32().min(0.1) * if self.paused { 0.0 } else { self.tempo };
        self.last = now;
        // Фаза — как в игре: на метр пути — по длине круга шага и бега.
        let rate = self.characters.as_ref().map_or_else(|| wind_render::character::stride_rate(self.speed), |c| c.stride_rate(self.speed));
        self.phase = (self.phase + self.speed * dt * rate) % std::f32::consts::TAU;
        if let Some(age) = &mut self.dead {
            *age += dt;
        }
        self.act_time += dt;
        self.clock += dt;
        self.editor.advance(dt);
        // Стойка — как в игре (см. `SceneBuilder::animate`).
        let (crouch_goal, prone_goal) = match self.stance {
            Stance::Stand => (0.0, 0.0),
            Stance::Crouch => (1.0, 0.0),
            Stance::Prone => (0.0, 1.0),
        };
        self.crouch += (crouch_goal - self.crouch).clamp(-dt * 5.0, dt * 5.0);
        let step = dt / wind_sim::world::GOING_PRONE;
        if prone_goal > self.prone {
            self.prone = (self.prone + step).min(1.0);
            self.rising = false;
        } else if prone_goal < self.prone {
            self.prone = (self.prone - step).max(0.0);
            self.rising = true;
        }
        if self.rwr_playing {
            let speed = self.rwr_selected().map_or(1.0, |a| a.speed);
            self.rwr_time += dt * speed;
        }
        self.view.set_joint_blend(self.blend);

        let center = self.center();
        let model = self.model.min(self.characters.as_ref().map_or(0, |c| c.models.len().saturating_sub(1)));
        let carry = if self.aiming || self.act == Act::Fire { 0.0 } else { wind_render::character::run_share(self.speed) };
        let motion = Motion { speed: self.speed, phase: self.phase, dir: Vec2::from_angle(self.heading.to_radians()), carry, crouch: self.crouch, prone: self.prone, rising: self.rising, clock: self.clock, lean: 0.0 };
        let action = self.action();
        // Редактор анимаций: солдат в позе клипа, без действий и падения.
        let editing = self.editor.on && self.dead.is_none();
        let edited = editing.then(|| self.characters.as_ref().and_then(|c| self.editor.pose(&c.models[model]))).flatten();
        let (action, weapon) = if editing { (None, self.weapon.filter(|_| self.editor.weapon)) } else { (action, self.weapon) };
        // Справка RWR: их анимация на соседнем солдате (номер 1), справа от нашего.
        let rwr_pose = self.rwr_on.then(|| self.rwr_selected().zip(self.characters.as_ref()).and_then(|(a, c)| c.models.get(model)?.pose_from_points(&a.sample(self.rwr_time)))).flatten();
        let mut overrides: Vec<(u32, Vec<Vec3>)> = Vec::new();
        if let Some(p) = edited {
            overrides.push((0, p));
        }
        if let Some(p) = rwr_pose.clone() {
            overrides.push((1, p));
        }
        self.view.set_pose_overrides(overrides);
        let id = 0;
        // Стреляют спереди: тело отлетает назад.
        let push = -Vec2::from_angle(self.facing) * if self.blast { wind_sim::world::BLAST_PUSH } else { wind_sim::world::BULLET_PUSH };
        let mut soldiers = Vec::new();
        let mut corpses = Vec::new();
        match self.dead {
            Some(age) => corpses.push(wind_render::CorpseDraw { id, pos: center, facing: self.facing, team: self.team, age, push, blast: self.blast, model: Some(model) }),
            None => soldiers.push(SceneSoldier {
                id,
                pos: center,
                facing: self.facing,
                team: self.team,
                player: false,
                look: Some(Look { model, weapon, motion, action }),
                stance: wind_sim::Stance::Stand,
                reloading: false,
                throwing: false,
                aiming: self.aiming,
                in_boat: false,
                // «Стрельба» — очередь: выстрел каждые 0,11 с, на каждый — отдача.
                shot_ago: if self.act == Act::Fire { self.act_time % 0.11 } else { f32::INFINITY },
            }),
        }
        if rwr_pose.is_some() {
            soldiers.push(SceneSoldier {
                id: 1,
                pos: self.rwr_spot(),
                facing: self.facing,
                team: 1 - self.team,
                player: false,
                look: Some(Look { model, weapon, motion: Motion::default(), action: None }),
                stance: wind_sim::Stance::Stand,
                reloading: false,
                throwing: false,
                aiming: true,
                in_boat: false,
                shot_ago: f32::INFINITY,
            });
        }
        let name = self.characters.as_ref().and_then(|c| c.models.get(model)).map_or("—", |m| m.name.as_str());
        let scene = Scene {
            title: f("Студия персонажей · {}", &[&name]),
            map: &self.map,
            bases: [center; 2],
            points: Vec::new(),
            capture_radius: 0.0,
            soldier_radius: 0.4,
            soldiers,
            bullets: Vec::new(),
            grenades: Vec::new(),
            blasts: Vec::new(),
            selected: None,
            flow: None,
            hud: None,
            kill_feed: Vec::new(),
            rings: Vec::new(),
            lines: Vec::new(),
            corpses,
            boats: Vec::new(),
            props: &[],
            atmosphere: self.atmosphere.clone(),
        };
        let rect = ui.max_rect();
        self.view.show(ui, &scene, true);
        let root = Mat4::from_translation(Vec3::new(center.x, 0.0, center.y)) * Mat4::from_rotation_y(-self.facing);
        if editing && let Some(set) = &self.characters {
            let cam = self.view.camera_3d();
            let view_proj = cam.view_proj(rect.width() / rect.height().max(1.0));
            self.editor.interact(ui, rect, view_proj, cam.eye(), root, &set.models[model]);
            return;
        }

        if self.skeleton
            && let Some(set) = &self.characters
        {
            let m = &set.models[model];
            // Точки скелета в мире, м: у живого — из позы, у убитого — тело-рэгдолл.
            let pose: Vec<Vec3> = match self.dead {
                Some(_) => self.view.corpse_points(id).map(<[Vec3]>::to_vec).unwrap_or_default(),
                None => {
                    let pose = m.pose(motion);
                    let pose = action.map_or(pose.clone(), |a| m.act(&pose, a, motion.prone >= 1.0).pose);
                    pose.iter().map(|&p| root.transform_point3(p * wind_render::rwr::VOXEL)).collect()
                }
            };
            let cam = self.view.camera_3d();
            let view_proj = cam.view_proj(rect.width() / rect.height().max(1.0));
            let to_screen = |p: Vec3| {
                let clip = view_proj * p.extend(1.0);
                let ndc = clip.xyz() / clip.w;
                pos2(rect.left() + (ndc.x * 0.5 + 0.5) * rect.width(), rect.top() + (0.5 - ndc.y * 0.5) * rect.height())
            };
            let painter = ui.painter_at(rect);
            let red = Color32::from_rgb(230, 40, 40);
            for &(a, b) in m.sticks.iter().filter(|&&(a, b)| a.max(b) < pose.len()) {
                painter.line_segment([to_screen(pose[a]), to_screen(pose[b])], Stroke::new(1.0, red));
            }
            for &p in &pose {
                painter.circle_filled(to_screen(p), 3.0, red);
            }
        }
    }
}

/// Что студия показывает руками.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Act {
    None,
    Reload,
    Throw,
    /// Очередь: отдача от каждого выстрела.
    Fire,
}
