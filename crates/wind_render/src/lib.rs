//! 3D-отрисовка мира на wgpu внутри кадра eframe.
//! Как устроено — заметка «Отрисовка — wgpu внутри eframe».

pub mod app_icon;
pub mod camera;
pub mod character;
mod decals;
mod effects;
pub mod gltf;
pub mod grass;
pub mod mapmesh;
pub mod mocap;
pub mod models;
pub mod props;
pub mod ragdoll;
pub mod renderer;
pub mod rwr;
pub mod scene;
pub mod voxel;

pub use camera::Camera;
pub use renderer::{DEPTH_FORMAT, FrameData, Installed, RenderStats, WorldPaint, install, stats};
pub use character::{Action, Motion, ReloadKind, THROW_TIME};
pub use glam;
pub use scene::{CorpseDraw, DrawInput, JOINT_BLEND, Look, PointDraw, SceneBuilder, SoldierDraw};
