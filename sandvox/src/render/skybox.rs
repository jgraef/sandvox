use std::path::PathBuf;

use bevy_ecs::{
    component::Component,
    entity::Entity,
    message::{
        Message,
        MessageReader,
    },
    name::NameOrEntity,
    query::Without,
    resource::Resource,
    schedule::{
        IntoScheduleConfigs,
        common_conditions::on_message,
    },
    system::{
        Commands,
        Populated,
        Res,
    },
};
use color_eyre::eyre::Error;
use image::RgbaImage;
use nalgebra::Vector2;
use wgpu::util::DeviceExt;

use crate::{
    ecs::{
        plugin::{
            Plugin,
            WorldBuilder,
        },
        schedule,
    },
    render::{
        RenderSystems,
        frame::{
            Frame,
            FrameBindGroupLayout,
        },
        surface::{
            RenderTarget,
            Surface,
        },
    },
    util::{
        format_size,
        image::{
            ImageLoadExt,
            ImageSizeExt,
        },
    },
    wgpu::WgpuContext,
};

#[derive(Clone, Copy, Debug, Default)]
pub struct SkyboxPlugin;

impl Plugin for SkyboxPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder
            .add_systems(
                schedule::Startup,
                (
                    create_pipeline_layout,
                    load_skybox
                        .after(create_pipeline_layout)
                        .run_if(on_message::<LoadSkybox>),
                )
                    .in_set(RenderSystems::Setup),
            )
            .add_systems(
                schedule::Render,
                (
                    (
                        create_pipeline,
                        load_skybox.run_if(on_message::<LoadSkybox>),
                    )
                        .in_set(RenderSystems::BeginFrame),
                    render_skybox.in_set(RenderSystems::RenderWorld),
                ),
            )
            .add_message::<LoadSkybox>();

        Ok(())
    }
}

// todo: this is not nice
#[derive(Clone, Debug, Message)]
pub struct LoadSkybox {
    pub path: PathBuf,
    pub entity: Entity,
}

#[derive(Clone, Debug, Component)]
pub struct Skybox {
    pub bind_group: wgpu::BindGroup,
}

#[derive(Debug, Resource)]
struct PipelineLayout {
    layout: wgpu::PipelineLayout,
    shader: wgpu::ShaderModule,
    bind_group_layout: wgpu::BindGroupLayout,
}

#[derive(Debug, Component)]
struct Pipeline {
    pipeline: wgpu::RenderPipeline,
}

fn create_pipeline_layout(
    wgpu: Res<WgpuContext>,
    frame_bind_group_layout: Res<FrameBindGroupLayout>,
    mut commands: Commands,
) {
    let bind_group_layout =
        wgpu.device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("skybox"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::Cube,
                        multisampled: false,
                    },
                    count: None,
                }],
            });

    let layout = wgpu
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("skybox"),
            bind_group_layouts: &[
                &frame_bind_group_layout.bind_group_layout,
                &bind_group_layout,
            ],
            immediate_size: 0,
        });

    let shader = wgpu
        .device
        .create_shader_module(wgpu::include_wgsl!("skybox.wgsl"));

    commands.insert_resource(PipelineLayout {
        layout,
        shader,
        bind_group_layout,
    });
}

fn create_pipeline(
    wgpu: Res<WgpuContext>,
    pipeline_layout: Res<PipelineLayout>,
    surfaces: Populated<(Entity, NameOrEntity, &Surface), Without<Pipeline>>,
    mut commands: Commands,
) {
    for (entity, name, surface) in surfaces {
        tracing::trace!(surface = %name, "creating skybox render pipeline for surface");

        let pipeline = wgpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("skybox"),
                layout: Some(&pipeline_layout.layout),
                vertex: wgpu::VertexState {
                    module: &pipeline_layout.shader,
                    entry_point: Some("skybox_vertex"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None,
                    unclipped_depth: false,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    conservative: false,
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: surface.depth_texture_format(),
                    depth_write_enabled: false,
                    depth_compare: wgpu::CompareFunction::Equal,
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &pipeline_layout.shader,
                    entry_point: Some("skybox_fragment"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: surface.surface_texture_format(),
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });

        commands.entity(entity).insert(Pipeline { pipeline });
    }
}

fn render_skybox(
    cameras: Populated<(&RenderTarget, &Skybox)>,
    mut frames: Populated<(&mut Frame, &Pipeline)>,
) {
    for (render_target, skybox) in cameras {
        if let Ok((mut frame, pipeline)) = frames.get_mut(render_target.0) {
            let render_pass = frame.render_pass_mut();

            render_pass.set_pipeline(&pipeline.pipeline);
            render_pass.set_bind_group(1, Some(&skybox.bind_group), &[]);
            render_pass.draw(0..3, 0..1);
        }
    }
}

fn load_skybox(
    mut load_messages: MessageReader<LoadSkybox>,
    wgpu: Res<WgpuContext>,
    layout: Res<PipelineLayout>,
    mut commands: Commands,
) {
    const FACES: [&str; 6] = ["right", "left", "top", "bottom", "front", "back"];

    for message in load_messages.read() {
        tracing::debug!(?message, "Loading skybox");

        let mut data = vec![];
        let mut size = Vector2::zeros();

        for (i, face) in FACES.into_iter().enumerate() {
            let path = message.path.join(format!("{face}.png"));
            let image = RgbaImage::from_path(path).unwrap();

            if i == 0 {
                size = image.size();
            }
            else {
                assert_eq!(image.size(), size);
            }

            data.extend(image.as_raw());
        }

        tracing::debug!(size = ?size, bytes = %format_size(data.len()), "skybox");

        let texture = wgpu.device.create_texture_with_data(
            &wgpu.queue,
            &wgpu::TextureDescriptor {
                label: Some("skybox"),
                size: wgpu::Extent3d {
                    width: size.x,
                    height: size.y,
                    depth_or_array_layers: 6,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            },
            wgpu::util::TextureDataOrder::LayerMajor,
            &data,
        );

        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("skybox"),
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..wgpu::TextureViewDescriptor::default()
        });

        let bind_group = wgpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("skybox"),
            layout: &layout.bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&texture_view),
            }],
        });

        commands
            .entity(message.entity)
            .insert(Skybox { bind_group });
    }
}
