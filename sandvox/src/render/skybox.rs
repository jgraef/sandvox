use std::path::Path;

use bevy_ecs::{
    change_detection::DetectChanges,
    component::Component,
    entity::Entity,
    hierarchy::Children,
    name::NameOrEntity,
    query::{
        Changed,
        Or,
        With,
        Without,
    },
    resource::Resource,
    schedule::{
        IntoScheduleConfigs,
        SystemCondition,
        common_conditions::any_match_filter,
    },
    system::{
        Commands,
        Populated,
        Query,
        Res,
        ResMut,
        Single,
    },
    world::Ref,
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
        atlas::AtlasHandle,
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
                    (
                        create_pipeline,
                        load_skybox,
                        update_skybox.run_if(
                            any_match_filter::<(Changed<GlobalTransform>, With<SkyboxBindGroup>)>
                                .or(any_match_filter::<(
                                    Or<(Changed<GlobalTransform>, Changed<Planet>)>,
                                    With<Planet>,
                                )>),
                        ),
                    )
                        .in_set(RenderSystems::BeginFrame),
                    render_skybox.in_set(RenderSystems::RenderWorld),
                ),
            );

        Ok(())
    }
}

#[derive(Clone, Debug, Component)]
pub struct Skybox {
    texture: wgpu::TextureView,
}

impl Skybox {
    pub fn load(wgpu: &WgpuContext, path: impl AsRef<Path>) -> Result<Self, Error> {
        // note: generate cube map from cylindrical: https://jaxry.github.io/panorama-to-cubemap/
        // layout: https://gpuweb.github.io/gpuweb/#texture-view-creation

        const FACES: [&str; 6] = ["px", "nx", "py", "ny", "pz", "nz"];
        let path = path.as_ref();

        tracing::debug!(?path, "Loading skybox");

        let mut data = vec![];
        let mut size = Vector2::zeros();

        for (i, face) in FACES.into_iter().enumerate() {
            profiling::scope!("load face");

            let path = path.join(format!("{face}.png"));
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

        let label = format!("skybox: {}", path.display());

        let texture = {
            profiling::scope!("create_texture");

            wgpu.device.create_texture_with_data(
                &wgpu.queue,
                &wgpu::TextureDescriptor {
                    label: Some(&label),
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
            )
        };

        let texture = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some(&label),
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..wgpu::TextureViewDescriptor::default()
        });

        Ok(Self { texture })
    }
}

#[derive(Clone, Debug, Component)]
pub struct Planet {
    pub texture: AtlasHandle,
    pub size: f32,
}

#[derive(Clone, Debug, Component)]
struct SkyboxBindGroup {
    bind_group: wgpu::BindGroup,
    data_buffer: wgpu::Buffer,
    num_planets: u32,
}

#[derive(Debug, Resource)]
struct PipelineLayout {
    layout: wgpu::PipelineLayout,
    shader: wgpu::ShaderModule,
    bind_group_layout: wgpu::BindGroupLayout,
}

#[derive(Debug, Component)]
struct Pipeline {
    skybox_pipeline: wgpu::RenderPipeline,
    planet_pipeline: wgpu::RenderPipeline,
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
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::Cube,
                            multisampled: false,
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

        let skybox_pipeline = wgpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("skybox/stars"),
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

        let planet_pipeline = wgpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("skybox/planets"),
                layout: Some(&pipeline_layout.layout),
                vertex: wgpu::VertexState {
                    module: &pipeline_layout.shader,
                    entry_point: Some("planet_vertex"),
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
                    entry_point: Some("planet_fragment"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: surface.surface_texture_format(),
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });

        commands.entity(entity).insert(Pipeline {
            skybox_pipeline,
            planet_pipeline,
        });
    }
}

#[profiling::function]
fn load_skybox(
    wgpu: Res<WgpuContext>,
    layout: Res<PipelineLayout>,
    skyboxes: Populated<
        (Entity, &Skybox, Option<&GlobalTransform>, Option<&Children>),
        Without<SkyboxBindGroup>,
    >,
    planets: Query<(&GlobalTransform, &Planet)>,
    mut commands: Commands,
) {
    for (entity, skybox, transform, children) in skyboxes {
        let mut data = transform.map_or_else(SkyboxData::default, SkyboxData::new);

        let mut num_planets = 0;

        for (planet_transform, planet) in children
            .into_iter()
            .flatten()
            .filter_map(|child| planets.get(*child).ok())
            .take(MAX_PLANETS)
        {
            data.planets[num_planets] = PlanetData::new(planet_transform, planet);
            num_planets += 1;
        }

        let data_buffer = wgpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("skybox"),
                contents: bytemuck::bytes_of(&data),
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
            });

        let bind_group = wgpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("skybox"),
            layout: &layout.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: data_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&skybox.texture),
                },
            ],
        });

        commands.entity(entity).insert(SkyboxBindGroup {
            bind_group,
            data_buffer,
            num_planets: num_planets.try_into().unwrap(),
        });
    }
}

#[profiling::function]
fn update_skybox(
    skyboxes: Populated<(
        &mut SkyboxBindGroup,
        Ref<GlobalTransform>,
        Option<&Children>,
    )>,
    planets: Query<(Ref<GlobalTransform>, Ref<Planet>)>,
    mut staging: ResMut<Staging>,
) {
    for (mut bind_group, skybox_transform, children) in skyboxes {
        let changed = skybox_transform.is_changed()
            || children
                .into_iter()
                .flatten()
                .filter_map(|child| planets.get(*child).ok())
                .any(|(planet_transform, planet)| {
                    planet_transform.is_changed() || planet.is_changed()
                });

        if changed {
            let mut data = SkyboxData::new(&skybox_transform);

            let mut num_planets = 0;

            for (planet_transform, planet) in children
                .into_iter()
                .flatten()
                .filter_map(|child| planets.get(*child).ok())
                .take(MAX_PLANETS)
            {
                data.planets[num_planets] = PlanetData::new(&planet_transform, &planet);
                num_planets += 1;
            }

            bind_group.num_planets = num_planets.try_into().unwrap();

            staging.write_buffer_from_slice(
                bind_group.data_buffer.slice(..),
                bytemuck::bytes_of(&data),
            );
        }
    }
}

#[profiling::function]
fn render_skybox(
    cameras: Populated<&RenderTarget>,
    mut frames: Populated<(&mut Frame, &Pipeline)>,
    bind_group: Single<&SkyboxBindGroup>,
) {
    // todo: associate skybox with camera that it renders to
    // we likely will do this after we refactor the render pass

    for render_target in cameras {
        if let Ok((mut frame, pipeline)) = frames.get_mut(render_target.0) {
            let frame = frame.active_mut();
            let span = frame.enter_span("skybox");

            frame
                .render_pass
                .set_bind_group(1, Some(&bind_group.bind_group), &[]);

            frame.render_pass.set_pipeline(&pipeline.skybox_pipeline);
            frame.render_pass.draw(0..3, 0..1);

            if bind_group.num_planets > 0 {
                frame.render_pass.set_pipeline(&pipeline.planet_pipeline);
                frame
                    .render_pass
                    .draw(0..(bind_group.num_planets * 6), 0..1);
            }

            frame.exit_span(span);
        }
    }
}

const MAX_PLANETS: usize = 2;

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
struct SkyboxData {
    model_matrix: Matrix4<f32>,
    planets: [PlanetData; MAX_PLANETS],
}

impl SkyboxData {
    fn new(transform: &GlobalTransform) -> Self {
        Self {
            model_matrix: transform.isometry.to_homogeneous(),
            planets: Zeroable::zeroed(),
        }
    }
}

impl Default for SkyboxData {
    fn default() -> Self {
        Self {
            model_matrix: Matrix4::identity(),
            planets: Zeroable::zeroed(),
        }
    }
}

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
struct PlanetData {
    model_matrix: Matrix4<f32>,
    texture_id: u32,
    scaling: f32,
    _padding: [u32; 2],
}

impl PlanetData {
    fn new(transform: &GlobalTransform, planet: &Planet) -> Self {
        Self {
            model_matrix: transform.isometry.to_homogeneous(),
            texture_id: planet.texture.id(),
            scaling: planet.size,
            _padding: Default::default(),
        }
    }
}
