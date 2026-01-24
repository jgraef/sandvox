pub mod atlas;
pub mod camera;
pub mod fps_counter;
pub mod frame;
pub mod mesh;
pub mod skybox;
pub mod staging;
pub mod surface;
pub mod text;

use std::path::PathBuf;

use bevy_ecs::{
    resource::Resource,
    schedule::{
        IntoScheduleConfigs,
        SystemSet,
        common_conditions::resource_changed,
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
            DefaultAtlas,
            begin_frames,
            create_default_resources,
            create_frame_bind_group_layout,
            create_frames,
            end_frames,
            update_frame_bind_groups,
            update_frame_uniform,
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
                    // initialize rendering
                    (
                        initialize_staging,
                        create_frame_bind_group_layout,
                        create_default_resources.after(initialize_staging),
                    )
                        .after(WgpuSystems::CreateContext)
                        .before(RenderSystems::Setup),
                    // update frame uniform
                    (update_frame_bind_groups, update_frame_uniform)
                        .after(RenderSystems::Setup)
                        .before(flush_staging),
                    // flush staging
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
                    (
                        update_frame_bind_groups.run_if(resource_changed::<DefaultAtlas>),
                        update_frame_uniform,
                    )
                        .in_set(RenderSystems::EndFrame)
                        .before(end_frames),
                    end_frames.in_set(RenderSystems::EndFrame),
                ),
            )
            .configure_system_sets(
                schedule::Startup,
                RenderSystems::Setup.after(WgpuSystems::CreateContext),
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

#[derive(Clone, Debug, Serialize, Deserialize, Resource)]
pub struct RenderConfig {
    #[serde(default = "default_true")]
    pub vsync: bool,

    #[serde(default = "default_font")]
    pub default_font: PathBuf,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            vsync: true,
            default_font: default_font(),
        }
    }
}

fn default_font() -> PathBuf {
    "assets/cozette.bdf".into()
}
