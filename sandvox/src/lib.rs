// used by crate::util::stats_alloc
#![feature(allocator_api)]

pub mod app;
pub mod build_info;
pub mod collide;
pub mod config;
pub mod ecs;
pub mod game;
pub mod input;
pub mod profiler;
#[cfg(feature = "rcon")]
pub mod rcon;
pub mod render;
pub mod sound;
pub mod ui;
pub mod util;
pub mod voxel;
pub mod wgpu;
