use std::path::PathBuf;

use bevy_ecs::{
    component::Component,
    entity::Entity,
    name::NameOrEntity,
    query::{
        Changed,
        Without,
    },
    resource::Resource,
    schedule::IntoScheduleConfigs,
    system::{
        Commands,
        Populated,
        Res,
        ResMut,
        Single,
    },
};
use bytemuck::{
    Pod,
    Zeroable,
};
use color_eyre::{
    Section,
    eyre::Error,
};
use image::RgbaImage;
use nalgebra::{
    Matrix4,
    Vector2,
};
use wgpu::util::DeviceExt;

use crate::{
    ecs::{
        plugin::{
            Plugin,
            WorldBuilder,
        },
        schedule,
        transform::GlobalTransform,
    },
    render::{
        RenderSystems,
        frame::{
            Frame,
            FrameBindGroupLayout,
        },
        staging::Staging,
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
    wgpu::{
        WgpuContext,
        buffer::WriteStaging,
    },
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
                    load_skybox.after(create_pipeline_layout),
                )
                    .in_set(RenderSystems::Setup),
            )
            .add_systems(
                schedule::Render,
                (
                    (create_pipeline, load_skybox, update_skybox).in_set(RenderSystems::BeginFrame),
                    render_skybox.in_set(RenderSystems::RenderWorld),
                ),
            );

        Ok(())
    }
}

#[derive(Clone, Debug, Component)]
pub struct Skybox {
    pub path: PathBuf,
}

#[derive(Clone, Debug, Component)]
struct SkyboxBindGroup {
    bind_group: wgpu::BindGroup,
    data_buffer: wgpu::Buffer,
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
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::Cube,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
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
                    depth_compare: wgpu::CompareFunction::LessEqual,
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

fn update_skybox(
    skyboxes: Populated<(&SkyboxBindGroup, &GlobalTransform), Changed<GlobalTransform>>,
    mut staging: ResMut<Staging>,
) {
    for (bind_group, transform) in skyboxes {
        let data = SkyboxData::new(transform);
        staging
            .write_buffer_from_slice(bind_group.data_buffer.slice(..), bytemuck::bytes_of(&data));
    }
}

fn render_skybox(
    cameras: Populated<&RenderTarget>,
    mut frames: Populated<(&mut Frame, &Pipeline)>,
    skybox: Single<&SkyboxBindGroup>,
) {
    for render_target in cameras {
        if let Ok((mut frame, pipeline)) = frames.get_mut(render_target.0) {
            let frame = frame.active_mut();
            let span = frame.enter_span("skybox");

            frame.render_pass.set_pipeline(&pipeline.pipeline);
            frame
                .render_pass
                .set_bind_group(1, Some(&skybox.bind_group), &[]);
            frame.render_pass.draw(0..3, 0..1);

            frame.exit_span(span);
        }
    }
}

fn load_skybox(
    wgpu: Res<WgpuContext>,
    layout: Res<PipelineLayout>,
    skyboxes: Populated<(Entity, &Skybox, Option<&GlobalTransform>), Without<SkyboxBindGroup>>,
    mut commands: Commands,
) {
    // note: generate cube map from cylindrical: https://jaxry.github.io/panorama-to-cubemap/

    // layout: https://gpuweb.github.io/gpuweb/#texture-view-creation

    //const FACES: [&str; 6] = ["right", "left", "top", "bottom", "front", "back"];
    const FACES: [&str; 6] = ["px", "nx", "py", "ny", "pz", "nz"];

    for (entity, skybox, transform) in skyboxes {
        tracing::debug!(?entity, path = %skybox.path.display(), "Loading skybox");

        let mut data = vec![];
        let mut size = Vector2::zeros();

        for (i, face) in FACES.into_iter().enumerate() {
            let path = skybox.path.join(format!("{face}.png"));
            let image = RgbaImage::from_path(&path)
                .with_note(|| path.display().to_string())
                .unwrap();

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

        let data_buffer = {
            let data = transform.map_or_else(SkyboxData::default, SkyboxData::new);

            wgpu.device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("skybox"),
                    contents: bytemuck::bytes_of(&data),
                    usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
                })
        };

        let bind_group = wgpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("skybox"),
            layout: &layout.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: data_buffer.as_entire_binding(),
                },
            ],
        });

        commands.entity(entity).insert(SkyboxBindGroup {
            bind_group,
            data_buffer,
        });
    }
}

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
struct SkyboxData {
    transform: Matrix4<f32>,
}

impl SkyboxData {
    fn new(transform: &GlobalTransform) -> Self {
        Self {
            transform: transform.isometry().to_homogeneous(),
        }
    }
}

impl Default for SkyboxData {
    fn default() -> Self {
        Self {
            transform: Matrix4::identity(),
        }
    }
}
