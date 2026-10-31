//! Студия персонажей: один солдат вблизи. Модель, оружие, шаг и бег, падение,
//! скелет поверх модели и ширина плавного сгиба у суставов — чтобы подбирать вид
//! солдата, не запуская бой.

use crate::icons::{self, Icon};
use crate::theme;
use crate::viewport::{Scene, SceneSoldier, Viewport};
use eframe::egui::{self, Color32, RichText, Stroke, pos2};
use std::sync::Arc;
use std::time::Instant;
use wind_render::character::CharacterSet;
use wind_render::glam::{Mat4, Vec3, Vec4Swizzles};
use wind_render::{Action, JOINT_BLEND, Look, Motion, THROW_TIME};
use wind_sim::Vec2;
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
}

impl Studio {
    pub fn new(characters: Option<Arc<CharacterSet>>, has_3d: bool) -> Self {
        let mut view = Viewport::new();
        view.has_3d = has_3d;
        view.set_characters(characters.clone());
        let center = Vec2::splat(GROUND as f32 * 0.5);
        view.aim_height = 0.9;
        view.look_at_3d(center, 3.5, CAMERA_PITCH);
        Studio {
            view,
            characters,
            map: Map::new(GROUND, GROUND),
            atmosphere: Atmosphere::default(),
            model: 0,
            weapon: Some(0),
            team: 0,
            speed: 2.0,
            phase: 0.0,
            heading: 0.0,
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
        }
    }

    fn center(&self) -> Vec2 {
        Vec2::splat(GROUND as f32 * 0.5)
    }

    /// Слева — модели и оружие.
    pub fn outliner(&mut self, ui: &mut egui::Ui) {
        let Some(set) = self.characters.clone() else {
            ui.label(RichText::new("Нет своих моделей: не найден data/factions.ron").color(theme::ERROR));
            return;
        };
        ui.label(RichText::new("Модели").strong());
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
        ui.label(RichText::new("Оружие").strong());
        if icons::selectable(ui, self.weapon.is_none(), Icon::Soldier, "без оружия", None).clicked() {
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
        theme::panel(ui, Icon::Run, "Анимация", true, |ui| {
            ui.horizontal(|ui| {
                for (name, speed) in [("Стоит", 0.0), ("Шаг", 2.0), ("Бег", 4.5)] {
                    if ui.selectable_label(self.dead.is_none() && self.speed == speed, name).clicked() {
                        self.speed = speed;
                        self.dead = None;
                    }
                }
                if ui.selectable_label(self.dead.is_some(), "Падает").clicked() {
                    self.dead = Some(0.0);
                }
            });
            ui.horizontal(|ui| {
                for (name, act) in [("Без действия", Act::None), ("Перезарядка", Act::Reload), ("Бросок гранаты", Act::Throw)] {
                    if ui.selectable_label(self.dead.is_none() && self.act == act, name).clicked() {
                        self.act = act;
                        self.act_time = 0.0;
                        self.dead = None;
                    }
                }
            });
            theme::props(ui, "studio_anim", |ui| {
                theme::label(ui, "Скорость");
                ui.add(egui::Slider::new(&mut self.speed, 0.0..=6.0).suffix(" м/с"));
                ui.end_row();
                theme::label(ui, "Замедление");
                ui.add(egui::Slider::new(&mut self.tempo, 0.05..=1.0).logarithmic(true).suffix("×"));
                ui.end_row();
                theme::label(ui, "Фаза шага");
                ui.add(egui::Slider::new(&mut self.phase, 0.0..=std::f32::consts::TAU).max_decimals(2));
                ui.end_row();
                theme::label(ui, "Направление");
                ui.add(egui::Slider::new(&mut self.heading, -180.0..=180.0).suffix("°").max_decimals(0))
                    .on_hover_text("Куда идёт относительно взгляда: 0 — вперёд, 90 — вправо, 180 — назад");
                ui.end_row();
                theme::label(ui, "Чем убит");
                ui.horizontal(|ui| {
                    // Выбрали — сразу падает заново.
                    if ui.selectable_value(&mut self.blast, false, "пулей").clicked() | ui.selectable_value(&mut self.blast, true, "взрывом").clicked() {
                        self.dead = Some(0.0);
                    }
                });
                ui.end_row();
            });
            let (icon, play) = if self.paused { (Icon::Play, "Дальше") } else { (Icon::Pause, "Пауза") };
            if icons::button(ui, icon, play).clicked() {
                self.paused = !self.paused;
            }
        });
        theme::panel(ui, Icon::Skeleton, "Сгиб у суставов", true, |ui| {
            theme::props(ui, "studio_blend", |ui| {
                theme::label(ui, "Ширина");
                ui.add(egui::Slider::new(&mut self.blend, 0.0..=12.0).suffix(" вокс."));
                ui.end_row();
            });
            ui.horizontal(|ui| {
                if ui.button("Жёстко (0)").clicked() {
                    self.blend = 0.0;
                }
                if ui.button(format!("Как в игре ({JOINT_BLEND})")).clicked() {
                    self.blend = JOINT_BLEND;
                }
            });
            ui.label(
                RichText::new("Воксель у сустава смешивает положение своей палки и соседней. В игре сгиб — как по умолчанию.")
                    .small()
                    .color(theme::TEXT_DIM),
            );
        });
        theme::panel(ui, Icon::Camera, "Вид", true, |ui| {
            theme::props(ui, "studio_view", |ui| {
                theme::label(ui, "Скелет");
                ui.checkbox(&mut self.skeleton, "поверх модели");
                ui.end_row();
                theme::label(ui, "Поворот");
                ui.drag_angle(&mut self.facing);
                ui.end_row();
                theme::label(ui, "Команда");
                ui.horizontal(|ui| {
                    for t in 0..2u8 {
                        ui.selectable_value(&mut self.team, t, RichText::new(theme::TEAM_NAME[t as usize]).color(theme::TEAM[t as usize]));
                    }
                });
                ui.end_row();
            });
            if icons::button(ui, Icon::Camera, "Камеру к солдату").clicked() {
                self.view.look_at_3d(self.center(), 3.5, CAMERA_PITCH);
            }
        });
        if let Some(set) = &self.characters
            && let Some(m) = set.models.get(self.model)
        {
            theme::panel(ui, Icon::Info, "Сведения", false, |ui| {
                let mesh = &set.meshes[m.mesh];
                theme::props(ui, "studio_info", |ui| {
                    theme::label(ui, "Палок");
                    ui.label(m.sticks.len().to_string());
                    ui.end_row();
                    theme::label(ui, "Точек");
                    ui.label(format!("{} / {} / {}", mesh.lods[0].points.len() / 4, mesh.lods[1].points.len() / 4, mesh.lods[2].points.len() / 4));
                    ui.end_row();
                    theme::label(ui, "Граней тени");
                    ui.label((mesh.indices.len() / 6).to_string());
                    ui.end_row();
                });
            });
        }
    }

    /// Действие руками сейчас: повторяется с паузой между повторами. Перезарядка —
    /// по выбранному оружию (как и сколько секунд).
    fn action(&self) -> Option<Action> {
        match self.act {
            Act::None => None,
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
        // Фаза — как в игре: на метр пути `stride_rate` (шагом круг 1,5 м, бегом 2,5 м).
        self.phase = (self.phase + self.speed * dt * wind_render::character::stride_rate(self.speed)) % std::f32::consts::TAU;
        if let Some(age) = &mut self.dead {
            *age += dt;
        }
        self.act_time += dt;
        self.view.set_joint_blend(self.blend);

        let center = self.center();
        let model = self.model.min(self.characters.as_ref().map_or(0, |c| c.models.len().saturating_sub(1)));
        let motion = Motion { speed: self.speed, phase: self.phase, dir: Vec2::from_angle(self.heading.to_radians()) };
        let action = self.action();
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
                look: Some(Look { model, weapon: self.weapon, motion, action }),
                prone: false,
                reloading: false,
                throwing: false,
            }),
        }
        let name = self.characters.as_ref().and_then(|c| c.models.get(model)).map_or("—", |m| m.name.as_str());
        let scene = Scene {
            title: format!("Студия персонажей · {name}"),
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
            atmosphere: self.atmosphere.clone(),
        };
        let rect = ui.max_rect();
        self.view.show(ui, &scene, true);

        if self.skeleton
            && let Some(set) = &self.characters
        {
            let m = &set.models[model];
            // Точки скелета в мире, м: у живого — из позы, у убитого — тело-рэгдолл.
            let root = Mat4::from_translation(Vec3::new(center.x, 0.0, center.y)) * Mat4::from_rotation_y(-self.facing);
            let pose: Vec<Vec3> = match self.dead {
                Some(_) => self.view.corpse_points(id).map(<[Vec3]>::to_vec).unwrap_or_default(),
                None => {
                    let pose = m.pose(motion);
                    let pose = action.map_or(pose.clone(), |a| m.act(&pose, a).pose);
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
}
