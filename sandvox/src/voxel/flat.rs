use std::{
    marker::PhantomData,
    num::NonZero,
};

use bevy_ecs::{
    component::Component,
    entity::Entity,
    lifecycle::HookContext,
    message::{
        Message,
        MessageReader,
    },
    name::NameOrEntity,
    query::Without,
    resource::Resource,
    schedule::IntoScheduleConfigs,
    system::{
        Commands,
        Local,
        Populated,
        Res,
    },
    world::DeferredWorld,
};
use color_eyre::eyre::Error;
use image::RgbaImage;
use morton_encoding::morton_decode;
use nalgebra::{
    Point3,
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
        RenderPipelineContext,
        RenderSystems,
        frame::Frame,
        surface::Surface,
    },
    util::image::ImageLoadExt,
    voxel::mesh::{
        BlockFace,
        Mesh,
        MeshBuilder,
        Vertex,
    },
    wgpu::{
        WgpuContext,
        image::ImageTextureExt,
    },
};

pub const CHUNK_SIDE_LENGTH_LOG2: u8 = 2;
pub const CHUNK_SIDE_LENGTH: u16 = 1 << CHUNK_SIDE_LENGTH_LOG2;
pub const CHUNK_NUM_VOXELS: usize = 1 << (3 * CHUNK_SIDE_LENGTH_LOG2);

pub struct FlatChunkPlugin<V> {
    _phantom: PhantomData<fn() -> V>,
}

impl<V> Default for FlatChunkPlugin<V> {
    fn default() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }
}

impl<V> Plugin for FlatChunkPlugin<V>
where
    V: IsOpaque + Send + Sync + 'static,
{
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder
            .add_message::<MeshChunkRequest>()
            .add_systems(
                schedule::Startup,
                create_render_pipeline_shared.after(RenderSystems::Setup),
            )
            .add_systems(schedule::PostUpdate, mesh_chunks::<V>)
            .add_systems(
                schedule::Render,
                (
                    create_render_pipelines_for_surfaces.in_set(RenderSystems::BeginFrame),
                    render_chunks.after(RenderSystems::BeginFrame),
                ),
            );
        Ok(())
    }
}

#[derive(Clone, Component)]
#[component(on_add = chunk_added, on_remove = chunk_removed)]
pub struct FlatChunk<V> {
    voxels: Box<[V; CHUNK_NUM_VOXELS]>,
}

impl<V> FlatChunk<V> {
    pub fn from_fn(mut f: impl FnMut(Point3<u16>) -> V) -> Self {
        let mut voxels = Box::new_uninit_slice(CHUNK_NUM_VOXELS);

        // fixme: memory leak when f panics
        for (i, voxel) in voxels.iter_mut().enumerate() {
            let point = Point3::from(morton_decode::<u16, 3>(i.try_into().unwrap()));
            voxel.write(f(point));
        }

        let voxels = unsafe { voxels.assume_init() };
        let voxels: Box<[V; CHUNK_NUM_VOXELS]> =
            voxels.try_into().unwrap_or_else(|_| unreachable!());

        Self { voxels }
    }
}

impl<V> FlatChunk<V>
where
    V: IsOpaque,
{
    pub fn naive_mesh(&self, mesh_builder: &mut MeshBuilder) {
        for (i, voxel) in self.voxels.iter().enumerate() {
            if voxel.is_opaque() {
                let point = Point3::from(morton_decode::<u16, 3>(i.try_into().unwrap()));

                for face in BlockFace::ALL {
                    let mut point = point;
                    match face {
                        BlockFace::Right => {
                            point.x += 1;
                        }
                        BlockFace::Up => {
                            point.y += 1;
                        }
                        BlockFace::Back => {
                            point.z += 1;
                        }
                        _ => {}
                    }

                    mesh_builder.push_quad(point, Vector2::repeat(1), face);
                }
            }
        }
    }
}

pub trait IsOpaque {
    fn is_opaque(&self) -> bool;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Message)]
struct MeshChunkRequest {
    entity: Entity,
}

#[derive(Debug, Resource)]
struct RenderPipelineShared {
    layout: wgpu::PipelineLayout,
    shader: wgpu::ShaderModule,
    material_bind_group: wgpu::BindGroup,
}

impl RenderPipelineShared {
    fn new(wgpu: &WgpuContext, pipeline_context: &RenderPipelineContext) -> Self {
        let material_bind_group_layout =
            wgpu.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("voxel::flat chunk"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                    ],
                });

        let layout = wgpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("voxel::flat"),
                bind_group_layouts: &[
                    &pipeline_context.camera_bind_group_layout,
                    &material_bind_group_layout,
                ],
                immediate_size: 0,
            });

        let material_bind_group = {
            let material_image = RgbaImage::from_path("assets/dirt.png").unwrap();

            let material_texture = wgpu.device.create_texture_with_data(
                &wgpu.queue,
                &material_image
                    .texture_descriptor(
                        "voxel::flat materials",
                        wgpu::TextureUsages::TEXTURE_BINDING,
                        const { NonZero::new(1).unwrap() },
                    )
                    .unwrap(),
                Default::default(),
                &material_image,
            );

            let material_texture_view =
                material_texture.create_view(&wgpu::TextureViewDescriptor {
                    label: Some("voxel::flat materials"),
                    ..Default::default()
                });

            let material_sampler = wgpu.device.create_sampler(&wgpu::SamplerDescriptor {
                label: Some("voxel::flat materials"),
                address_mode_u: wgpu::AddressMode::Repeat,
                address_mode_v: wgpu::AddressMode::Repeat,
                address_mode_w: wgpu::AddressMode::Repeat,
                mag_filter: wgpu::FilterMode::Nearest,
                min_filter: wgpu::FilterMode::Nearest,
                mipmap_filter: wgpu::MipmapFilterMode::Nearest,
                ..Default::default()
            });

            wgpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("voxel::flat materials"),
                layout: &material_bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&material_texture_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&material_sampler),
                    },
                ],
            })
        };

        let shader = wgpu
            .device
            .create_shader_module(wgpu::include_wgsl!("flat.wgsl"));

        Self {
            layout,
            shader,
            material_bind_group,
        }
    }
}

#[derive(Debug, Component)]
struct RenderPipelinePerSurface {
    pipeline: wgpu::RenderPipeline,
}

impl RenderPipelinePerSurface {
    fn new(wgpu: &WgpuContext, shared: &RenderPipelineShared, surface: &Surface) -> Self {
        let pipeline = wgpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("voxel::flat"),
                layout: Some(&shared.layout),
                vertex: wgpu::VertexState {
                    module: &shared.shader,
                    entry_point: Some("vertex_main"),
                    compilation_options: Default::default(),
                    buffers: &[Vertex::LAYOUT],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: Some(wgpu::Face::Back),
                    unclipped_depth: false,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    conservative: false,
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: surface.depth_texture_format(),
                    depth_write_enabled: true,
                    depth_compare: wgpu::CompareFunction::Less,
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &shared.shader,
                    entry_point: Some("fragment_main"),
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

        Self { pipeline }
    }
}

fn chunk_added(mut world: DeferredWorld, context: HookContext) {
    tracing::debug!(entity = ?context.entity, "chunk added");

    world.write_message(MeshChunkRequest {
        entity: context.entity,
    });
}

fn chunk_removed(mut world: DeferredWorld, context: HookContext) {
    let mut commands = world.commands();
    let mut entity = commands.entity(context.entity);
    entity.try_remove::<ChunkMesh>();
}

fn create_render_pipeline_shared(
    wgpu: Res<WgpuContext>,
    pipeline_context: Res<RenderPipelineContext>,
    mut commands: Commands,
) {
    commands.insert_resource(RenderPipelineShared::new(&wgpu, &pipeline_context));
}

#[derive(Debug, Component)]
struct ChunkMesh {
    mesh: Option<Mesh>,
}

fn mesh_chunks<V>(
    wgpu: Res<WgpuContext>,
    mut requests: MessageReader<MeshChunkRequest>,
    chunk_data: Populated<&FlatChunk<V>>,
    mut commands: Commands,
    mut mesh_builder: Local<MeshBuilder>,
) where
    V: IsOpaque + Send + Sync + 'static,
{
    for request in requests.read() {
        tracing::debug!(entity = ?request.entity, "meshing chunk");

        if let Ok(chunk) = chunk_data.get(request.entity) {
            chunk.naive_mesh(&mut mesh_builder);
            let mesh = mesh_builder.finish(&wgpu, "chunk");

            // debug
            assert!(mesh.is_some(), "chunk is empty");

            commands.entity(request.entity).insert(ChunkMesh { mesh });
            mesh_builder.clear();
        }
        else {
            tracing::warn!(entity = ?request.entity, "requested chunk to be meshed, but it doesn't have chunk data");
        }
    }
}

fn create_render_pipelines_for_surfaces(
    wgpu: Res<WgpuContext>,
    shared: Res<RenderPipelineShared>,
    surfaces: Populated<(Entity, NameOrEntity, &Surface), Without<RenderPipelinePerSurface>>,
    mut commands: Commands,
) {
    for (entity, name, surface) in surfaces {
        tracing::trace!(surface = %name, "creating voxel::flat render pipeline for surface");

        commands
            .entity(entity)
            .insert(RenderPipelinePerSurface::new(&wgpu, &shared, surface));
    }
}

fn render_chunks(
    shared: Res<RenderPipelineShared>,
    frames: Populated<(&mut Frame, &RenderPipelinePerSurface)>,
    chunk_meshes: Populated<(&ChunkMesh, &GlobalTransform)>,
) {
    for (mut frame, pipeline) in frames {
        let mut render_pass = frame.render_pass_mut();

        render_pass.set_pipeline(&pipeline.pipeline);
        render_pass.set_bind_group(1, Some(&shared.material_bind_group), &[]);

        let mut count = 0;
        for (mesh, transform) in &chunk_meshes {
            // todo: bind transform
            let _ = transform;

            if let Some(mesh) = &mesh.mesh {
                mesh.draw(&mut render_pass, 0, 0..1);
                count += 1;
            }
        }

        tracing::trace!("rendered {count} chunk meshes");
    }
}
