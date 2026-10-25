//! Сборка кадра на CPU: позы солдат, эффекты, свет, изменённые чанки карты.
//! Источник данных (локальная симуляция или сеть) не важен — на входе простые списки.

use crate::camera::{Camera, to_3d};
use crate::mapmesh::MapMesher;
use crate::models::{MeshId, SOLDIER_VOXEL};
use crate::renderer::{FrameData, Globals, GroundPoint, Instance, MAX_POINTS, PendingMap, SHADOW_SIZE};
use std::sync::{Arc, Mutex};
use glam::camera::rh::{proj::directx as proj, view};
use glam::{Mat4, Vec2, Vec3};
use wind_sim::map::{CHUNK, Map};

pub struct SoldierDraw {
    pub id: u32,
    pub pos: Vec2,
    pub facing: f32,
    pub team: u8,
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
    anim: Vec<Anim>,
    /// Цвета команд (sRGB 0..1).
    pub team_colors: [[f32; 3]; 2],
}

impl Default for SceneBuilder {
    fn default() -> Self {
        SceneBuilder {
            mesher: MapMesher::default(),
            pending: Arc::default(),
            anim: Vec::new(),
            team_colors: [[0.33, 0.52, 0.78], [0.78, 0.33, 0.30]],
        }
    }
}

/// Насколько далеко от центра экрана ещё есть смысл рисовать, м.
fn view_radius(camera: &Camera) -> f32 {
    camera.distance * 2.2 + 30.0
}

impl SceneBuilder {
    pub fn build(&mut self, input: &DrawInput) -> FrameData {
        let cam = input.camera;
        let reach = view_radius(cam);
        let reach2 = reach * reach;

        // --- Карта.
        let update = self.mesher.update(input.map, input.map_key);
        if update.reset || !update.chunks.is_empty() {
            let mut pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
            if update.reset {
                pending.reset = true;
                pending.chunks.clear();
            }
            pending.chunks.extend(update.chunks);
        }
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
        let v = SOLDIER_VOXEL;
        for s in input.soldiers {
            let speed = self.animate(s, input.dt);
            if s.pos.distance_squared(cam.target) > reach2 {
                continue;
            }
            let a = self.anim[s.id as usize];
            let tint = {
                let c = self.team_colors[s.team as usize & 1];
                [c[0], c[1], c[2], 0.0]
            };
            let swing = a.phase.sin() * 0.7 * (speed / 4.5).min(1.0);
            let bob = (a.phase * 2.0).sin().abs() * 0.5 * v * (speed / 4.5).min(1.0);
            let root = Mat4::from_translation(to_3d(s.pos, bob)) * Mat4::from_rotation_y(-s.facing);
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
            let m = Mat4::from_translation(to_3d(head - dir * 0.9, 1.25))
                * Mat4::from_rotation_y(-angle)
                * Mat4::from_scale(Vec3::new(1.8, 0.05, 0.05));
            cube.push(Instance { model: m.to_cols_array_2d(), tint: [1.0, 0.86, 0.45, 1.0] });
        }
        for &p in input.grenades {
            let m = Mat4::from_translation(to_3d(p, 0.12)) * Mat4::from_scale(Vec3::splat(0.24));
            cube.push(Instance { model: m.to_cols_array_2d(), tint: [0.30, 0.36, 0.22, 0.5] });
        }
        for &(p, radius, age) in input.blasts {
            // Растёт, потом опадает. Взрыв — из кубиков, как и всё остальное.
            let t = (age / 0.6).clamp(0.0, 1.0);
            let size = radius * (0.3 + 0.7 * t) * (1.0 - t * t);
            if size <= 0.01 {
                continue;
            }
            let m = Mat4::from_translation(to_3d(p, size * 0.3)) * Mat4::from_scale(Vec3::new(size, size * 0.6, size));
            cube.push(Instance { model: m.to_cols_array_2d(), tint: [1.0, 0.55 + 0.35 * (1.0 - t), 0.2, 1.0] });
        }

        let mut instances = Vec::new();
        let mut batches = Vec::new();
        for (mesh, list) in by_mesh.iter().enumerate() {
            batches.push((mesh, instances.len() as u32, list.len() as u32));
            instances.extend_from_slice(list);
        }

        FrameData {
            globals: self.globals(input),
            instances,
            batches,
            map: self.pending.clone(),
            visible_chunks,
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
        let sun = Vec3::new(0.35, 0.85, 0.40).normalize();
        let center = to_3d(cam.target, 0.0);

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
        for p in input.points.iter().take(MAX_POINTS - 1) {
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

        let eye = cam.eye();
        Globals {
            view_proj: cam.view_proj(input.aspect).to_cols_array_2d(),
            light_view_proj: light.to_cols_array_2d(),
            sun_dir: [sun.x, sun.y, sun.z, 0.0],
            sun_color: [1.6, 1.52, 1.36, 0.8],
            ambient_sky: [0.42, 0.48, 0.58, 0.0],
            // w — с какого расстояния от глаза начинается туман.
            ambient_ground: [0.22, 0.20, 0.17, cam.distance * 1.15],
            fog: [0.62, 0.68, 0.72, 0.004],
            eye: [eye.x, eye.y, eye.z, 1.0],
            params: [input.map.width as f32, input.time, count as f32, 1.0 / SHADOW_SIZE as f32],
            points,
        }
    }
}
