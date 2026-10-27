//! Сборка кадра на CPU: позы солдат, эффекты, свет, изменённые чанки карты.
//! Источник данных (локальная симуляция или сеть) не важен — на входе простые списки.

use crate::camera::{Camera, to_3d};
use crate::character::{CharacterSet, LODS, Motion};
use crate::decals::Decals;
use crate::effects::Effects;
use wind_sim::atmosphere::Atmosphere;
use crate::mapmesh::MapMesher;
use crate::models::{MeshId, SOLDIER_VOXEL};
use crate::renderer::{FrameData, Globals, GroundPoint, Instance, MAX_POINTS, PendingMap, SHADOW_SIZE, SkinInstance};
use std::sync::{Arc, Mutex};
use glam::camera::rh::{proj::directx as proj, view};
use glam::{Mat4, Vec2, Vec3};
use wind_sim::map::{CHUNK, Map};

pub struct SoldierDraw {
    pub id: u32,
    pub pos: Vec2,
    pub facing: f32,
    pub team: u8,
    /// Явный вид — для студии. Без него модель выбирается по номеру солдата,
    /// а шаг — по тому, как он перемещается.
    pub look: Option<Look>,
}

/// Модель, оружие и движение солдата, заданные явно.
#[derive(Clone, Copy, Debug)]
pub struct Look {
    pub model: usize,
    pub weapon: Option<usize>,
    pub motion: Motion,
}

/// Убитый солдат: лежит, пока не возродится.
pub struct CorpseDraw {
    pub id: u32,
    pub pos: Vec2,
    pub facing: f32,
    pub team: u8,
    /// Сколько секунд прошло со смерти — для падения.
    pub age: f32,
    /// Явная модель — для студии.
    pub model: Option<usize>,
}

pub struct PointDraw {
    pub pos: Vec2,
    pub radius: f32,
    pub owner: Option<u8>,
}

pub struct DrawInput<'a> {
    pub map: &'a Map,
    /// Различает карты: при смене ключа меши карты строятся заново.
    pub map_key: u64,
    pub camera: &'a Camera,
    pub aspect: f32,
    pub soldiers: &'a [SoldierDraw],
    /// Пули: (голова, скорость).
    pub bullets: &'a [(Vec2, Vec2)],
    pub grenades: &'a [Vec2],
    /// Взрывы: (центр, радиус, возраст в секундах).
    pub blasts: &'a [(Vec2, f32, f32)],
    pub points: &'a [PointDraw],
    /// Кольцо под своим солдатом.
    pub marker: Option<Vec2>,
    /// Атмосфера карты: свет, туман, цвет, обводка.
    pub atmosphere: &'a Atmosphere,
    /// Где вьюпорт в пикселях кадра: x, y, ширина, высота — для обводки.
    pub viewport_px: [f32; 4],
    /// Дополнительные кольца на земле: (центр, радиус, цвет sRGB) — например, кисть редактора.
    pub rings: &'a [(Vec2, f32, [f32; 3])],
    pub corpses: &'a [CorpseDraw],
    /// Наша команда: силуэты сквозь стены — только у неё. `None` — видно всех.
    pub own_team: Option<u8>,
    pub time: f32,
    /// Время с прошлого кадра, с — для анимации шага.
    pub dt: f32,
}

#[derive(Clone, Copy, Default)]
struct Anim {
    phase: f32,
    speed: f32,
    last: Vec2,
    seen: bool,
}

pub struct SceneBuilder {
    mesher: MapMesher,
    /// Меши карты, которые ждут отправки на GPU.
    pending: Arc<Mutex<PendingMap>>,
    /// Для какой карты и версии покрытия земли уже отправлена текстура.
    ground_key: Option<(u64, u64)>,
    /// Следы боя и для какой карты они.
    decals: Decals,
    /// Обломки, пыль и дым.
    effects: Effects,
    decals_key: Option<u64>,
    anim: Vec<Anim>,
    /// Цвета команд (sRGB 0..1).
    pub team_colors: [[f32; 3]; 2],
    /// Свои солдаты (из `install`). Без них рисуются встроенные болванчики.
    pub characters: Option<Arc<CharacterSet>>,
    /// Ширина плавного сгиба у суставов, воксели. 0 — части жёсткие.
    pub joint_blend: f32,
}

/// Ширина сгиба по умолчанию, воксели.
pub const JOINT_BLEND: f32 = 4.0;

impl Default for SceneBuilder {
    fn default() -> Self {
        SceneBuilder {
            mesher: MapMesher::default(),
            pending: Arc::default(),
            ground_key: None,
            decals: Decals::default(),
            effects: Effects::default(),
            decals_key: None,
            anim: Vec::new(),
            team_colors: [[0.33, 0.52, 0.78], [0.78, 0.33, 0.30]],
            characters: None,
            joint_blend: JOINT_BLEND,
        }
    }
}

/// Куда падает убитый: у каждого солдата своё направление, но всегда одно и то же.
pub fn falls_forward(id: u32) -> bool {
    id.wrapping_mul(0x9E37_79B9) >> 31 == 1
}

/// Насколько далеко от центра экрана ещё есть смысл рисовать, м.
fn view_radius(camera: &Camera) -> f32 {
    camera.distance * 2.2 + 30.0
}

impl SceneBuilder {
    pub fn with_characters(characters: Option<Arc<CharacterSet>>) -> Self {
        SceneBuilder { characters, ..Default::default() }
    }

    pub fn build(&mut self, input: &DrawInput) -> FrameData {
        let cam = input.camera;
        let reach = view_radius(cam);
        let reach2 = reach * reach;

        // --- Карта.
        let update = self.mesher.update(input.map, input.map_key);
        if update.reset {
            self.effects.reset();
        }
        self.effects.update(input.map, input.dt.min(0.1), input.blasts, &update.broken);
        let ground_key = (input.map_key, input.map.ground_revision);
        let ground_changed = update.reset || self.ground_key != Some(ground_key);
        if update.reset || !update.chunks.is_empty() || ground_changed {
            let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            if update.reset {
                pending.reset = true;
                pending.chunks.clear();
            }
            pending.chunks.extend(update.chunks);
            if ground_changed {
                self.ground_key = Some(ground_key);
                pending.ground = Some((input.map.width, input.map.height, input.map.ground.clone()));
            }
        }
        self.battle_traces(input);
        let cx = input.map.chunks_x();
        let half = CHUNK as f32 * 0.5;
        let visible_chunks = (0..input.map.chunk_revisions.len() as u32)
            .filter(|&c| {
                let center = Vec2::new(((c % cx) * CHUNK) as f32 + half, ((c / cx) * CHUNK) as f32 + half);
                center.distance(cam.target) < reach + half * 1.5
            })
            .collect();

        // --- Экземпляры, по мешам.
        let mut by_mesh: [Vec<Instance>; crate::models::MESH_COUNT] = Default::default();
        let characters = self.characters.clone();
        // Свои солдаты: на каждого — один экземпляр и матрицы всех палок.
        // По мешу и уровню детализации: индекс = меш × LODS + уровень.
        let mut by_char: Vec<Vec<SkinInstance>> = vec![Vec::new(); characters.as_ref().map_or(0, |c| c.meshes.len() * LODS)];
        let mut bones: Vec<[[f32; 4]; 4]> = Vec::new();
        let eye = cam.eye();
        // Уровень детализации — по тому, сколько пикселей занимает воксель при 1080p:
        // точка всё равно не меньше 2,6 пикселя, и мелкие воксели вдали сливаются.
        let focal_1080 = 1080.0 / (2.0 * (cam.fov_y * 0.5).tan());
        let lod_at = |p: Vec3| {
            let voxel_px = crate::rwr::VOXEL * focal_1080 / p.distance(eye).max(0.1);
            if voxel_px > 0.6 {
                0
            } else if voxel_px > 0.3 {
                1
            } else {
                2
            }
        };
        // Цвет команды — для силуэта сквозь стены.
        let team_colors = self.team_colors.map(|c| u32::from_le_bytes([c[0], c[1], c[2], 1.0].map(|v| (v * 255.0) as u8)));
        // Признаки: бит 0 — оружие, бит 1 — виден силуэт сквозь стены (только у своих:
        // врагов за стеной видеть нельзя; наблюдатель видит всех).
        let own_team = input.own_team;
        let skin = |base: usize, team: u8, weapon: bool| {
            let ghost = own_team.is_none_or(|t| t == team);
            SkinInstance { base: base as u32, flags: weapon as u32 | (ghost as u32) << 1, team: team_colors[team as usize & 1], _pad: 0 }
        };
        let v = SOLDIER_VOXEL;
        for s in input.soldiers {
            let speed = self.animate(s, input.dt);
            if s.pos.distance_squared(cam.target) > reach2 {
                continue;
            }
            let a = self.anim[s.id as usize];
            // Свои модели: поза по скелету, оружие в правой руке.
            let look = s.look.or_else(|| {
                let (model, weapon) = characters.as_ref()?.pick(s.team, s.id)?;
                Some(Look { model, weapon: Some(weapon), motion: Motion { speed, phase: a.phase * 2.0 } })
            });
            if let Some(set) = &characters
                && let Some(look) = look
            {
                let model = &set.models[look.model];
                let pose = model.pose(look.motion);
                let at = to_3d(s.pos, input.map.height_at(s.pos));
                let lod = lod_at(at);
                let root = Mat4::from_translation(at) * Mat4::from_rotation_y(-s.facing);
                by_char[model.mesh * LODS + lod].push(skin(bones.len(), s.team, false));
                model.bones(&pose, &mut |m| bones.push((root * m).to_cols_array_2d()));
                if let Some(weapon) = look.weapon
                    && let Some(grip) = model.weapon_frame(&pose)
                {
                    by_char[set.weapons[weapon].mesh * LODS + lod].push(skin(bones.len(), s.team, true));
                    bones.push((root * grip).to_cols_array_2d());
                }
                continue;
            }
            let tint = {
                let c = self.team_colors[s.team as usize & 1];
                [c[0], c[1], c[2], 0.0]
            };
            let swing = a.phase.sin() * 0.7 * (speed / 4.5).min(1.0);
            let bob = (a.phase * 2.0).sin().abs() * 0.5 * v * (speed / 4.5).min(1.0);
            let root = Mat4::from_translation(to_3d(s.pos, input.map.height_at(s.pos) + bob)) * Mat4::from_rotation_y(-s.facing);
            let part = |offset: Vec3, rot_z: f32| {
                let m = root * Mat4::from_translation(offset * v) * Mat4::from_rotation_z(rot_z);
                Instance { model: m.to_cols_array_2d(), tint }
            };
            by_mesh[MeshId::Leg as usize].push(part(Vec3::new(0.0, 8.0, -1.5), swing));
            by_mesh[MeshId::Leg as usize].push(part(Vec3::new(0.0, 8.0, 1.5), -swing));
            by_mesh[MeshId::Torso as usize].push(part(Vec3::new(0.0, 8.0, 0.0), 0.0));
            by_mesh[MeshId::Head as usize].push(part(Vec3::new(0.0, 15.0, 0.0), 0.0));
            // Руки вытянуты вперёд — держат винтовку.
            by_mesh[MeshId::Arm as usize].push(part(Vec3::new(0.0, 14.0, -4.5), 1.05));
            by_mesh[MeshId::Arm as usize].push(part(Vec3::new(0.0, 14.0, 4.5), 1.35));
            by_mesh[MeshId::Rifle as usize].push(part(Vec3::new(3.5, 10.5, 2.0), 0.0));
        }

        let cube = &mut by_mesh[MeshId::Cube as usize];
        for &(head, vel) in input.bullets {
            if head.distance_squared(cam.target) > reach2 {
                continue;
            }
            let dir = vel.normalize_or_zero();
            let angle = dir.y.atan2(dir.x);
            let m = Mat4::from_translation(to_3d(head - dir * 0.9, input.map.height_at(head) + 1.25))
                * Mat4::from_rotation_y(-angle)
                * Mat4::from_scale(Vec3::new(1.8, 0.05, 0.05));
            cube.push(Instance { model: m.to_cols_array_2d(), tint: [1.0, 0.86, 0.45, 1.0] });
        }
        for &p in input.grenades {
            let m = Mat4::from_translation(to_3d(p, input.map.height_at(p) + 0.12)) * Mat4::from_scale(Vec3::splat(0.24));
            cube.push(Instance { model: m.to_cols_array_2d(), tint: [0.30, 0.36, 0.22, 0.5] });
        }
        for &(p, radius, age) in input.blasts {
            // Растёт, потом опадает. Взрыв — из кубиков, как и всё остальное.
            let t = (age / 0.6).clamp(0.0, 1.0);
            let size = radius * (0.3 + 0.7 * t) * (1.0 - t * t);
            if size <= 0.01 {
                continue;
            }
            let m = Mat4::from_translation(to_3d(p, input.map.height_at(p) + size * 0.3)) * Mat4::from_scale(Vec3::new(size, size * 0.6, size));
            cube.push(Instance { model: m.to_cols_array_2d(), tint: [1.0, 0.55 + 0.35 * (1.0 - t), 0.2, 1.0] });
        }

        self.effects.debris_instances(&mut by_mesh[MeshId::Cube as usize]);

        // Убитые: падают и лежат, пока солдат не возродится.
        if let Some(set) = &characters {
            for c in input.corpses {
                if c.pos.distance_squared(cam.target) > reach2 {
                    continue;
                }
                let Some(model) = c.model.or_else(|| set.pick(c.team, c.id).map(|p| p.0)) else { continue };
                let model = &set.models[model];
                let pose = model.corpse(c.age, falls_forward(c.id));
                let at = to_3d(c.pos, input.map.height_at(c.pos));
                let lod = lod_at(at);
                let root = Mat4::from_translation(at) * Mat4::from_rotation_y(-c.facing);
                by_char[model.mesh * LODS + lod].push(skin(bones.len(), c.team, false));
                model.bones(&pose, &mut |m| bones.push((root * m).to_cols_array_2d()));
            }
        }

        let mut instances = Vec::new();
        let mut batches = Vec::new();
        for (mesh, list) in by_mesh.iter().enumerate() {
            batches.push((mesh, instances.len() as u32, list.len() as u32));
            instances.extend_from_slice(list);
        }
        let mut skins = Vec::new();
        let mut char_batches = Vec::new();
        for (mesh, list) in by_char.iter().enumerate() {
            if !list.is_empty() {
                char_batches.push((mesh / LODS, mesh % LODS, skins.len() as u32, list.len() as u32));
                skins.extend_from_slice(list);
            }
        }

        FrameData {
            globals: self.globals(input),
            instances,
            batches,
            char_batches,
            skins,
            bones,
            particles: self.effects.particles(cam.eye()),
            map: self.pending.clone(),
            visible_chunks,
        }
    }

    /// Новые подпалины и лужи крови — сразу в очередь на GPU.
    fn battle_traces(&mut self, input: &DrawInput) {
        let mut pending = None;
        if self.decals_key != Some(input.map_key) {
            self.decals_key = Some(input.map_key);
            let size = self.decals.reset(input.map.width, input.map.height);
            let p = pending.get_or_insert_with(|| self.pending.lock().unwrap_or_else(|e| e.into_inner()));
            p.decal_size = Some(size);
            p.decal_rects.clear();
        }
        let mut rects = Vec::new();
        for &(c, r, _) in input.blasts {
            rects.extend(self.decals.blast(c, r, input.time));
        }
        // Кровь — когда тело уже упало.
        for c in input.corpses.iter().filter(|c| c.age > 0.45) {
            // Лужа — под корпусом: тело упало на метр вперёд или назад от ног.
            let fall = Vec2::from_angle(c.facing) * if falls_forward(c.id) { 1.0 } else { -1.0 };
            rects.extend(self.decals.corpse(c.id, c.pos + fall));
        }
        if rects.is_empty() {
            return;
        }
        let p = pending.get_or_insert_with(|| self.pending.lock().unwrap_or_else(|e| e.into_inner()));
        for r in rects {
            p.decal_rects.push((r, self.decals.rect(r)));
        }
    }

    /// Двигает фазу шага по пройденному пути. Возвращает сглаженную скорость.
    fn animate(&mut self, s: &SoldierDraw, dt: f32) -> f32 {
        let i = s.id as usize;
        if self.anim.len() <= i {
            self.anim.resize(i + 1, Anim::default());
        }
        let a = &mut self.anim[i];
        let moved = s.pos.distance(a.last);
        if !a.seen || moved > 3.0 {
            // Первый кадр или телепорт (возрождение).
            *a = Anim { phase: 0.0, speed: 0.0, last: s.pos, seen: true };
            return 0.0;
        }
        let speed = if dt > 0.0 { (moved / dt).min(10.0) } else { a.speed };
        a.speed += (speed - a.speed) * 0.25;
        a.phase += moved * 2.4;
        a.last = s.pos;
        a.speed
    }

    fn globals(&self, input: &DrawInput) -> Globals {
        let cam = input.camera;
        let atmo = input.atmosphere;
        let sun = Vec3::from_array(atmo.sun_direction()).normalize();
        // Цвета атмосферы заданы в sRGB, а свет считается в линейных — переводим.
        let lin = |c: [f32; 3], k: f32| c.map(|v| v.max(0.0).powf(2.2) * k);
        let sun_lin = lin(atmo.sun_color, atmo.sun_strength);
        let sky_lin = lin(atmo.sky_color, 0.8);
        let shadow_lin = lin(atmo.shadow_color, 0.9);
        let center = to_3d(cam.target, cam.target_height);

        // Камера теней: ортографическая, вокруг центра экрана. Сдвиг кратен
        // текселю — иначе края теней дрожат при движении камеры.
        let extent = (cam.distance * 1.1).clamp(25.0, 220.0);
        let view = view::look_at_mat4(center + sun * 300.0, center, Vec3::Y);
        let texel = 2.0 * extent / SHADOW_SIZE as f32;
        let c = view.transform_point3(center);
        let snap = Vec3::new((c.x / texel).round() * texel - c.x, (c.y / texel).round() * texel - c.y, 0.0);
        let light = proj::orthographic(-extent, extent, -extent, extent, 1.0, 600.0) * Mat4::from_translation(snap) * view;

        let mut points = [GroundPoint { center_radius: [0.0; 4], color: [0.0; 4] }; MAX_POINTS];
        let mut count = 0;
        for p in input.points.iter().take(MAX_POINTS - 1 - input.rings.len().min(MAX_POINTS - 1)) {
            let color = p.owner.map_or([0.85, 0.85, 0.82], |t| self.team_colors[t as usize & 1]);
            points[count] = GroundPoint {
                center_radius: [p.pos.x, p.pos.y, p.radius, 0.0],
                color: [color[0], color[1], color[2], 0.12],
            };
            count += 1;
        }
        if let Some(m) = input.marker {
            points[count] = GroundPoint { center_radius: [m.x, m.y, 0.75, 0.0], color: [1.0, 0.63, 0.16, 0.0] };
            count += 1;
        }
        for &(c, r, color) in input.rings {
            if count == MAX_POINTS {
                break;
            }
            points[count] = GroundPoint { center_radius: [c.x, c.y, r, 0.0], color: [color[0], color[1], color[2], 0.0] };
            count += 1;
        }

        let eye = cam.eye();
        Globals {
            view_proj: cam.view_proj(input.aspect).to_cols_array_2d(),
            light_view_proj: light.to_cols_array_2d(),
            sun_dir: [sun.x, sun.y, sun.z, 0.0],
            sun_color: [sun_lin[0], sun_lin[1], sun_lin[2], 0.85],
            ambient_sky: [sky_lin[0], sky_lin[1], sky_lin[2], 0.0],
            // w — с какого расстояния от глаза начинается туман.
            ambient_ground: [shadow_lin[0], shadow_lin[1], shadow_lin[2], cam.distance * 1.15],
            fog: [atmo.sky_color[0], atmo.sky_color[1], atmo.sky_color[2], atmo.fog],
            eye: [eye.x, eye.y, eye.z, 1.0],
            params: [input.map.width as f32, input.time, count as f32, 1.0 / SHADOW_SIZE as f32],
            view: cam.view().to_cols_array_2d(),
            grade: [atmo.saturation, atmo.contrast, atmo.brightness, atmo.toon],
            // w — фокусное расстояние в пикселях: радиус затенения в метрах → пиксели.
            tint: [atmo.tint[0], atmo.tint[1], atmo.tint[2], input.viewport_px[3] / (2.0 * (cam.fov_y * 0.5).tan())],
            post: [atmo.outline, atmo.occlusion, atmo.vignette, cam.far()],
            screen: input.viewport_px,
            skin: [self.joint_blend, 0.0, 0.0, 0.0],
            points,
        }
    }
}
