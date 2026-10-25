//! 3D-отрисовка мира на wgpu внутри кадра eframe.
//! Как устроено — заметка «Отрисовка — wgpu внутри eframe».

pub mod camera;
pub mod mapmesh;
pub mod models;
pub mod renderer;
pub mod scene;
pub mod voxel;

pub use camera::Camera;
pub use renderer::{DEPTH_FORMAT, FrameData, RenderStats, WorldPaint, install, stats};
pub use scene::{DrawInput, PointDraw, SceneBuilder, SoldierDraw};
