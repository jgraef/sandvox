pub mod camera;
pub mod camera_controller;
pub mod frame;
pub mod mesh;
pub mod surface;
pub mod texture_atlas;

use bevy_ecs::schedule::{
    IntoScheduleConfigs,
    SystemSet,
};
use color_eyre::eyre::Error;

use crate::{
    ecs::{
        plugin::{
            Plugin,
            WorldBuilder,
        },
        schedule,
    },
    render::surface::handle_window_events,
    wgpu::WgpuSystems,
};

#[derive(Clone, Debug, Default)]
pub struct RenderPlugin {
    // todo config
}

impl Plugin for RenderPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder
            .add_systems(
                schedule::Render,
                (
                    handle_window_events.before(RenderSystems::BeginFrame),
                    frame::begin_frame.in_set(RenderSystems::BeginFrame),
                    frame::end_frame
                        .in_set(RenderSystems::EndFrame)
                        .after(RenderSystems::BeginFrame),
                ),
            )
            .configure_system_sets(
                schedule::Startup,
                RenderSystems::Setup.after(WgpuSystems::CreateContext),
            );

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, SystemSet, PartialEq, Eq, Hash)]
pub enum RenderSystems {
    Setup,
    BeginFrame,
    EndFrame,
}
