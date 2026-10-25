//! Симуляция боя. Ничего не знает про окно, GPU и звук:
//! один и тот же код крутится и на сервере без окна, и в лаборатории.

pub mod config;
pub mod grid;
pub mod map;
pub mod mapgen;
pub mod nav;
pub mod rng;
pub mod world;

mod ai;
mod combat;

pub use config::SimConfig;
pub use glam::Vec2;
pub use world::{
    CapturePoint, Controller, Event, PlayerInput, Projectile, ProjectileKind, Soldier, TickStats,
    World,
};
