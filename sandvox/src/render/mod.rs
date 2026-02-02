pub mod atlas;
pub mod camera;
pub mod command;
pub mod fps_counter;
pub mod mesh;
pub mod pass;
pub mod render_target;
pub mod shadow_map;
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
    },
    system::{
        Commands,
        Res,
        ResMut,
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
        atlas::Atlas,
        command::RenderFunctions,
        pass::{
            context::{
                PendingCommandBuffers,
                flush_command_buffers,
            },
            main_pass::{
                MainPassPlugin,
                MainPassSystems,
            },
            ui_pass::UiPassSystems,
        },
        staging::{
            Staging,
            flush_staging,
            initialize_staging,
        },
        surface::{
            create_surfaces,
            present_surfaces,
            reconfigure_surfaces,
            set_swap_chain_texture,
        },
        text::Font,
    },
    util::serde::default_true,
    wgpu::{
        WgpuContext,
        WgpuPlugin,
        WgpuSystems,
    },
};

#[derive(Clone, Debug, Default)]
pub struct RenderPlugin {
    pub config: RenderConfig,
}

impl Plugin for RenderPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder
            .require_plugin::<WgpuPlugin>()
            .add_plugin(MainPassPlugin)?
            // create resources
            .insert_resource(self.config.clone())
            .init_resource::<PendingCommandBuffers>()
            // startup systems
            .add_systems(
                schedule::Startup,
                (
                    // initialize rendering
                    (
                        initialize_staging,
                        create_default_resources.after(initialize_staging),
                    )
                        .after(WgpuSystems::CreateContext)
                        .before(RenderSystems::Setup),
                    // flush staging
                    flush_staging.after(RenderSystems::Setup),
                ),
            )
            // render systems
            .add_systems(
                schedule::Render,
                (
                    (create_surfaces, reconfigure_surfaces).before(RenderSystems::BeginFrame),
                    set_swap_chain_texture
                        .after(create_surfaces)
                        .after(reconfigure_surfaces)
                        .before(RenderSystems::Render),
                    (flush_command_buffers, present_surfaces)
                        .chain()
                        .after(RenderSystems::EndFrame),
                ),
            )
            .configure_system_sets(
                schedule::Startup,
                RenderSystems::Setup.after(WgpuSystems::CreateContext),
            )
            .configure_system_sets(
                schedule::Render,
                MainPassSystems::Render.before(UiPassSystems::Render),
            )
            .configure_system_sets(
                schedule::Render,
                RenderSystems::EndFrame.after(RenderSystems::BeginFrame),
            );

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, SystemSet, PartialEq, Eq, Hash)]
pub enum RenderSystems {
    Setup,
    BeginFrame,
    Render,
    EndFrame,
}

#[derive(Clone, Debug, Serialize, Deserialize, Resource)]
pub struct RenderConfig {
    #[serde(default = "default_true")]
    pub vsync: bool,

    #[serde(default = "default_font")]
    pub default_font: PathBuf,

    /// FOV in degrees
    ///
    /// # TODO
    ///
    /// We think this doesn't really belong in the renderer config. We wanted to
    /// put it into [`GraphicsConfig`][crate::config::GraphicsConfig] first, but
    /// moved it here, because we then have convenient access to it.
    #[serde(default = "default_fov")]
    pub fov: f32,

    #[serde(default)]
    pub depth_prepass: bool,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            vsync: true,
            default_font: default_font(),
            fov: default_fov(),
            depth_prepass: false,
        }
    }
}

fn default_font() -> PathBuf {
    "assets/cozette.bdf".into()
}

fn default_fov() -> f32 {
    60.0
}

#[profiling::function]
fn create_default_resources(
    wgpu: Res<WgpuContext>,
    config: Res<RenderConfig>,
    mut commands: Commands,
    mut staging: ResMut<Staging>,
) {
    let sampler = wgpu.device.create_sampler(&Default::default());

    let atlas = Atlas::new(&wgpu.device, Default::default());

    let font = Font::open(&config.default_font, &wgpu.device, &mut *staging).unwrap_or_else(|e| {
        panic!(
            "Error while loading font: {e}: {}",
            config.default_font.display()
        )
    });

    commands.insert_resource(DefaultSampler(sampler));
    commands.insert_resource(DefaultAtlas(atlas));
    commands.insert_resource(DefaultFont(font));
}

// todo: make this a resource that contains all the samplers we use
#[derive(Clone, Debug, Resource)]
pub struct DefaultSampler(pub wgpu::Sampler);

#[derive(Debug, Resource, derive_more::Deref, derive_more::DerefMut)]
pub struct DefaultAtlas(pub Atlas);

#[derive(Debug, Resource, derive_more::Deref, derive_more::DerefMut)]
pub struct DefaultFont(pub Font);
