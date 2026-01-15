pub mod camera;
pub mod fps_counter;
pub mod frame;
pub mod mesh;
pub mod surface;
pub mod texture_atlas;

use bevy_ecs::{
    resource::Resource,
    schedule::{
        IntoScheduleConfigs,
        SystemSet,
    },
};
use color_eyre::eyre::Error;
use serde::{
    Deserialize,
    Serialize,
};

use crate::{
    ecs::{
        plugin::{
            Plugin,
            WorldBuilder,
        },
        schedule,
    },
    render::surface::handle_window_events,
    util::serde::default_true,
    wgpu::WgpuSystems,
};

#[derive(Clone, Debug, Default)]
pub struct RenderPlugin {
    pub config: RenderConfig,
}

impl Plugin for RenderPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder
            .add_systems(
                schedule::Render,
                (
                    handle_window_events.before(RenderSystems::BeginFrame),
                    frame::begin_frame.in_set(RenderSystems::BeginFrame),
                    frame::end_frame.in_set(RenderSystems::EndFrame),
                ),
            )
            .configure_system_sets(
                schedule::Startup,
                RenderSystems::Setup.after(WgpuSystems::CreateContext),
            )
            .configure_system_sets(
                schedule::Render,
                RenderSystems::RenderFrame.after(RenderSystems::BeginFrame),
            )
            .configure_system_sets(
                schedule::Render,
                RenderSystems::RenderFrame.before(RenderSystems::EndFrame),
            )
            .configure_system_sets(
                schedule::Render,
                RenderSystems::EndFrame.after(RenderSystems::BeginFrame),
            )
            .insert_resource(self.config.clone());

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, SystemSet, PartialEq, Eq, Hash)]
pub enum RenderSystems {
    Setup,
    BeginFrame,
    RenderFrame,
    EndFrame,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, Resource)]
pub struct RenderConfig {
    #[serde(default = "default_true")]
    pub vsync: bool,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self { vsync: true }
    }
}
