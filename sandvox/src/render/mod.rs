pub mod camera;
pub mod fps_counter;
pub mod frame;
pub mod mesh;
pub mod staging;
pub mod surface;
pub mod text;
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
    render::{
        frame::{
            begin_frames,
            create_frame_bind_group_layout,
            create_frames,
            end_frames,
        },
        staging::{
            flush_staging,
            initialize_staging,
        },
        surface::{
            create_surfaces,
            reconfigure_surfaces,
            update_viewports,
        },
    },
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
                schedule::Startup,
                (
                    (initialize_staging, create_frame_bind_group_layout)
                        .after(WgpuSystems::CreateContext)
                        .before(RenderSystems::Setup),
                    flush_staging.after(RenderSystems::Setup),
                ),
            )
            .add_systems(
                schedule::Render,
                (
                    (update_viewports, create_surfaces, reconfigure_surfaces)
                        .before(RenderSystems::BeginFrame),
                    (create_frames, begin_frames)
                        .chain()
                        .in_set(RenderSystems::BeginFrame),
                    end_frames.in_set(RenderSystems::EndFrame),
                ),
            )
            .configure_system_sets(
                schedule::Render,
                RenderSystems::RenderWorld
                    .after(RenderSystems::BeginFrame)
                    .before(RenderSystems::EndFrame)
                    .before(RenderSystems::RenderUi),
            )
            .configure_system_sets(
                schedule::Render,
                RenderSystems::RenderUi
                    .after(RenderSystems::BeginFrame)
                    .before(RenderSystems::EndFrame)
                    .after(RenderSystems::RenderWorld),
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
    RenderWorld,
    RenderUi,
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
