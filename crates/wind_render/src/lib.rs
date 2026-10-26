//! 3D-отрисовка мира на wgpu внутри кадра eframe.
//! Как устроено — заметка «Отрисовка — wgpu внутри eframe».

pub mod camera;
pub mod character;
mod decals;
mod effects;
pub mod mapmesh;
pub mod models;
pub mod renderer;
pub mod rwr;
pub mod scene;
pub mod voxel;

pub use camera::Camera;
pub use renderer::{DEPTH_FORMAT, FrameData, Installed, RenderStats, WorldPaint, install, stats};
pub use character::Motion;
pub use glam;
pub use scene::{CorpseDraw, DrawInput, JOINT_BLEND, Look, PointDraw, SceneBuilder, SoldierDraw, falls_forward};
