//! Камера. В игре — как в RWR: сверху под углом, север (−Y симуляции) вверху экрана.
//! В инструментах её же можно вращать, как в Blender.
//!
//! Координаты: симуляция (x, y) → 3D (x, высота, y). Ось Y в 3D — вверх.

use glam::camera::rh::{proj::directx as proj, view};
use glam::{Mat4, Vec2, Vec3, Vec4Swizzles};

#[derive(Clone, Debug)]
pub struct Camera {
    /// Куда смотрим, координаты симуляции.
    pub target: Vec2,
    /// Расстояние от цели, м.
    pub distance: f32,
    /// Наклон над горизонтом, рад. π/2 — строго сверху.
    pub pitch: f32,
    /// Поворот вокруг вертикали, рад. 0 — север вверху экрана.
    pub yaw: f32,
    /// Вертикальный угол обзора, рад.
    pub fov_y: f32,
    /// Высота земли под целью, м — чтобы на холме камера смотрела на холм, а не в него.
    pub target_height: f32,
}

impl Default for Camera {
    /// Как в RWR (`scene.xml`): 36 м, наклон ≈ 58°, поворот ≈ 17° (улицы идут по диагонали),
    /// угол обзора 45°.
    fn default() -> Self {
        Camera { target: Vec2::ZERO, distance: 36.0, pitch: 1.02, yaw: -0.29, fov_y: 0.785, target_height: 0.0 }
    }
}

pub fn to_3d(p: Vec2, height: f32) -> Vec3 {
    Vec3::new(p.x, height, p.y)
}

/// Пирамида видимости камеры: шесть плоскостей, нормали внутрь. По ней отбирается то, что
/// попадает на экран, — рисовать и считать позы тех, кого не видно, незачем.
#[derive(Clone, Copy, Debug)]
pub struct Frustum {
    planes: [glam::Vec4; 6],
}

impl Frustum {
    /// Из матрицы «мир → экран» (глубина 0…1, как у wgpu).
    pub fn new(view_proj: Mat4) -> Self {
        let (r0, r1, r2, r3) = (view_proj.row(0), view_proj.row(1), view_proj.row(2), view_proj.row(3));
        let planes = [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r2, r3 - r2].map(|p| p / p.truncate().length().max(1e-6));
        Frustum { planes }
    }

    /// Шар с центром `center` и радиусом `radius` хотя бы краем попадает в кадр.
    /// (С запасом: у углов пирамиды может пропустить и невидимое — это не страшно.)
    pub fn sees(&self, center: Vec3, radius: f32) -> bool {
        self.planes.iter().all(|p| p.truncate().dot(center) + p.w >= -radius)
    }
}

impl Camera {
    pub fn frustum(&self, aspect: f32) -> Frustum {
        Frustum::new(self.view_proj(aspect))
    }

    pub fn eye(&self) -> Vec3 {
        let (sp, cp) = self.pitch.sin_cos();
        let (sy, cy) = self.yaw.sin_cos();
        to_3d(self.target, self.target_height) + Vec3::new(sy * cp, sp, cy * cp) * self.distance
    }

    pub fn far(&self) -> f32 {
        self.distance * 6.0 + 200.0
    }

    pub fn view(&self) -> Mat4 {
        view::look_at_mat4(self.eye(), to_3d(self.target, self.target_height), Vec3::Y)
    }

    pub fn view_proj(&self, aspect: f32) -> Mat4 {
        proj::perspective(self.fov_y, aspect.max(0.01), 0.5, self.far()) * self.view()
    }

    /// «Вверх» и «вправо» экрана на земле — чтобы WASD шли по экрану, а не по сторонам света.
    pub fn screen_axes(&self) -> (Vec2, Vec2) {
        let (sy, cy) = self.yaw.sin_cos();
        (-Vec2::new(sy, cy), Vec2::new(cy, -sy))
    }

    /// Точка рельефа под курсором: несколько уточнений — пересекаем луч с плоскостью
    /// на высоте земли в найденной точке, пока не сойдётся.
    pub fn pick(&self, ndc: Vec2, aspect: f32, map: &wind_sim::map::Map) -> Option<Vec2> {
        let mut p = self.ground_point(ndc, aspect, self.target_height)?;
        for _ in 0..4 {
            p = self.ground_point(ndc, aspect, map.height_at(p))?;
        }
        Some(p)
    }

    /// Точка на земле под курсором. `ndc` — координаты экрана от −1 до 1 (Y вверх).
    pub fn ground_point(&self, ndc: Vec2, aspect: f32, height: f32) -> Option<Vec2> {
        let inv = self.view_proj(aspect).inverse();
        let near = inv * glam::Vec4::new(ndc.x, ndc.y, 0.0, 1.0);
        let far = inv * glam::Vec4::new(ndc.x, ndc.y, 1.0, 1.0);
        let (near, far) = (near.xyz() / near.w, far.xyz() / far.w);
        let dir = far - near;
        if dir.y.abs() < 1e-6 {
            return None;
        }
        let t = (height - near.y) / dir.y;
        (t > 0.0).then(|| {
            let p = near + dir * t;
            Vec2::new(p.x, p.z)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frustum_sees_what_is_on_screen() {
        let cam = Camera { target: Vec2::new(100.0, 100.0), ..Default::default() };
        let (aspect, f) = (1.6, cam.frustum(1.6));
        let vp = cam.view_proj(aspect);
        let mut inside = 0;
        for y in 0..200 {
            for x in 0..200 {
                let p = to_3d(Vec2::new(x as f32, y as f32), 0.0);
                let ndc = vp.project_point3(p);
                let on_screen = ndc.x.abs() <= 1.0 && ndc.y.abs() <= 1.0 && (0.0..=1.0).contains(&ndc.z);
                // Всё, что на экране, — видит; то, что дальше 3 м от экрана, — нет.
                assert!(!on_screen || f.sees(p, 0.0), "{p} на экране, но отброшена");
                if f.sees(p, 0.0) {
                    inside += 1;
                    assert!(ndc.x.abs() <= 1.001 && ndc.y.abs() <= 1.001, "{p} не на экране, но оставлена");
                }
            }
        }
        // На экране — малая часть площадки 200 × 200 м, а не круг в сотню метров.
        assert!(inside > 500 && inside < 6000, "точек в кадре: {inside}");
        assert!(f.sees(to_3d(cam.target, 0.0), 0.0));
        assert!(!f.sees(to_3d(cam.target + Vec2::new(90.0, 0.0), 0.0), 3.0));
    }

    #[test]
    fn screen_center_hits_target() {
        let cam = Camera { target: Vec2::new(100.0, 50.0), ..Default::default() };
        let p = cam.ground_point(Vec2::ZERO, 1.6, 0.0).unwrap();
        assert!(p.distance(cam.target) < 0.01, "{p}");
        // Верх экрана — север (меньший y симуляции).
        let up = cam.ground_point(Vec2::new(0.0, 0.5), 1.6, 0.0).unwrap();
        assert!(up.y < cam.target.y);
        let right = cam.ground_point(Vec2::new(0.5, 0.0), 1.6, 0.0).unwrap();
        assert!(right.x > cam.target.x);
    }

    #[test]
    fn target_projects_to_center() {
        let cam = Camera { target: Vec2::new(256.0, 256.0), distance: 300.0, ..Default::default() };
        let p = cam.view_proj(1.7).project_point3(to_3d(cam.target, 0.0));
        assert!(p.x.abs() < 1e-3 && p.y.abs() < 1e-3 && (0.0..=1.0).contains(&p.z), "{p}");
    }
}
