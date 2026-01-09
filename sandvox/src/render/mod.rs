pub mod camera;
pub mod camera_controller;
pub mod frame;
pub mod surface;

use bevy_ecs::{
    resource::Resource,
    schedule::{
        IntoScheduleConfigs,
        SystemSet,
    },
    system::{
        Commands,
        Res,
    },
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
    wgpu::WgpuContext,
};

#[derive(Clone, Debug, Default)]
pub struct RenderPlugin {
    // todo config
}

impl Plugin for RenderPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder
            .add_systems(
                schedule::Startup,
                setup_renderer.in_set(RenderSystems::Setup),
            )
            .add_systems(
                schedule::Render,
                (
                    handle_window_events.before(RenderSystems::BeginFrame),
                    frame::begin_frame.in_set(RenderSystems::BeginFrame),
                    frame::end_frame
                        .in_set(RenderSystems::EndFrame)
                        .after(RenderSystems::BeginFrame),
                ),
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

#[derive(Debug, Resource)]
pub struct RenderPipelineContext {
    pub camera_bind_group_layout: wgpu::BindGroupLayout,
}

fn setup_renderer(wgpu: Res<WgpuContext>, mut commands: Commands) {
    let camera_bind_group_layout =
        wgpu.device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("camera"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

    commands.insert_resource(RenderPipelineContext {
        camera_bind_group_layout,
    });
}
