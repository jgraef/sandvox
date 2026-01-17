use std::ops::Range;

use bevy_ecs::{
    component::Component,
    entity::Entity,
    name::NameOrEntity,
    query::{
        Added,
        Changed,
        Or,
        With,
        Without,
    },
    resource::Resource,
    schedule::{
        IntoScheduleConfigs,
        SystemCondition,
        common_conditions::{
            any_match_filter,
            resource_exists,
        },
    },
    system::{
        Commands,
        Local,
        Populated,
        Query,
        Res,
        ResMut,
    },
};
use bytemuck::{
    Pod,
    Zeroable,
};
use color_eyre::eyre::Error;
use nalgebra::{
    Matrix4,
    Point2,
    Vector4,
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
        camera::{
            CameraFrustrum,
            FrustrumCulled,
        },
        frame::{
            Frame,
            FrameUniformLayout,
        },
        surface::{
            AttachedCamera,
            Surface,
        },
        texture_atlas::{
            Atlas,
            AtlasSystems,
        },
    },
    wgpu::{
        WgpuContext,
        WgpuContextBuilder,
        WgpuSystems,
        buffer::{
            WriteStaging,
            WriteStagingCommit,
            WriteStagingTransaction,
        },
    },
};

#[derive(Clone, Copy, Debug)]
pub struct MeshPlugin;

impl Plugin for MeshPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder
            .add_systems(
                schedule::Startup,
                request_wgpu_features.in_set(WgpuSystems::RequestFeatures),
            )
            .add_systems(
                schedule::Startup,
                create_mesh_render_pipeline_shared
                    .after(RenderSystems::Setup)
                    .after(AtlasSystems::BuildAtlas),
            )
            .add_systems(
                schedule::Render,
                (
                    create_mesh_render_pipeline_for_surfaces.in_set(RenderSystems::BeginFrame),
                    update_instance_buffer
                        .in_set(RenderSystems::BeginFrame)
                        .run_if(
                            any_match_filter::<(
                                With<Mesh>,
                                Or<(
                                    Changed<GlobalTransform>,
                                    Added<GlobalTransform>,
                                    Added<Mesh>,
                                )>,
                            )>,
                        ),
                    render_meshes
                        .in_set(RenderSystems::RenderWorld)
                        .run_if(resource_exists::<InstanceBuffer>),
                    render_wireframes
                        .in_set(RenderSystems::RenderWorld)
                        .run_if(
                            resource_exists::<InstanceBuffer>
                                .and(resource_exists::<RenderWireframes>),
                        )
                        .after(render_meshes),
                ),
            );
        Ok(())
    }
}

fn request_wgpu_features(mut builder: ResMut<WgpuContextBuilder>) {
    builder.request_features(wgpu::Features::POLYGON_MODE_LINE);
}

#[derive(Clone, Copy, Debug, Default, Resource)]
pub struct RenderWireframes;

#[derive(Clone, Debug, Default)]
pub struct MeshBuilder {
    vertices: Vec<Vertex>,
    faces: Vec<[u32; 3]>,
}

impl MeshBuilder {
    pub fn clear(&mut self) {
        self.vertices.clear();
        self.faces.clear();
    }

    pub fn push(
        &mut self,
        vertices: impl IntoIterator<Item = Vertex>,
        faces: impl IntoIterator<Item = [u32; 3]>,
    ) {
        let base_index: u32 = self.vertices.len().try_into().unwrap();

        self.vertices.extend(vertices);
        self.faces.extend(
            faces
                .into_iter()
                .map(|face| face.map(|index| index + base_index)),
        );
    }

    pub fn finish(&self, wgpu: &WgpuContext, label: &str) -> Option<Mesh> {
        if self.faces.is_empty() {
            None
        }
        else {
            assert!(!self.vertices.is_empty());

            let vertex_buffer = wgpu
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(&format!("{label} vertices")),
                    contents: bytemuck::cast_slice(&self.vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });

            let index_buffer = wgpu
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(&format!("{label} indices")),
                    contents: bytemuck::cast_slice(&self.faces),
                    usage: wgpu::BufferUsages::INDEX,
                });

            let num_indices = 3 * u32::try_from(self.faces.len()).unwrap();

            Some(Mesh {
                vertex_buffer,
                index_buffer,
                indices: 0..num_indices,
                base_vertex: 0,
            })
        }
    }
}

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
struct Instance {
    model_matrix: Matrix4<f32>,
}

impl Instance {
    pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: size_of::<Self>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Instance,
        attributes: &wgpu::vertex_attr_array![
            4 => Float32x4,
            5 => Float32x4,
            6 => Float32x4,
            7 => Float32x4,
        ],
    };
}

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
pub struct Vertex {
    pub position: Vector4<f32>,
    pub normal: Vector4<f32>,
    pub uv: Point2<f32>,
    pub texture_id: u32,
}

impl Vertex {
    pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: size_of::<Self>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![
            0 => Float32x4,
            1 => Float32x4,
            2 => Float32x2,
            3 => Uint32,
        ],
    };
}

#[derive(Clone, Debug, Component)]
pub struct Mesh {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub indices: Range<u32>,
    pub base_vertex: i32,
}

impl Mesh {
    pub const INDEX_FORMAT: wgpu::IndexFormat = wgpu::IndexFormat::Uint32;

    pub fn draw(
        &self,
        render_pass: &mut wgpu::RenderPass,
        vertex_buffer_slot: u32,
        instances: Range<u32>,
    ) {
        render_pass.set_vertex_buffer(vertex_buffer_slot, self.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), Self::INDEX_FORMAT);
        render_pass.draw_indexed(self.indices.clone(), self.base_vertex, instances);
    }
}

#[derive(Debug, Resource)]
struct InstanceBuffer {
    buffer: wgpu::Buffer,
    size: u64,
}

#[derive(Clone, Copy, Debug, Component)]
struct InstanceId(u32);

#[derive(Debug, Resource)]
struct MeshRenderPipelineShared {
    layout: wgpu::PipelineLayout,
    shader: wgpu::ShaderModule,
}

#[derive(Debug, Component)]
struct MeshRenderPipelinePerSurface {
    pipeline: wgpu::RenderPipeline,
    wireframe_pipeline: wgpu::RenderPipeline,
}

fn create_mesh_render_pipeline_shared(
    wgpu: Res<WgpuContext>,
    frame_uniform_layout: Res<FrameUniformLayout>,
    atlas: Res<Atlas>,
    mut commands: Commands,
) {
    let layout = wgpu
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mesh"),
            bind_group_layouts: &[
                &frame_uniform_layout.bind_group_layout,
                &atlas.bind_group_layout(),
            ],
            immediate_size: 0,
        });

    let shader = wgpu
        .device
        .create_shader_module(wgpu::include_wgsl!("mesh.wgsl"));

    commands.insert_resource(MeshRenderPipelineShared { layout, shader });
}

fn create_mesh_render_pipeline_for_surfaces(
    wgpu: Res<WgpuContext>,
    shared: Res<MeshRenderPipelineShared>,
    surfaces: Populated<(Entity, NameOrEntity, &Surface), Without<MeshRenderPipelinePerSurface>>,
    mut commands: Commands,
) {
    for (entity, name, surface) in surfaces {
        tracing::trace!(surface = %name, "creating mesh render pipeline for surface");

        let pipeline = wgpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("mesh"),
                layout: Some(&shared.layout),
                vertex: wgpu::VertexState {
                    module: &shared.shader,
                    entry_point: Some("vertex_main"),
                    compilation_options: Default::default(),
                    buffers: &[Vertex::LAYOUT, Instance::LAYOUT],
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

        let wireframe_pipeline =
            wgpu.device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("wireframe mesh"),
                    layout: Some(&shared.layout),
                    vertex: wgpu::VertexState {
                        module: &shared.shader,
                        entry_point: Some("vertex_main_wireframe"),
                        compilation_options: Default::default(),
                        buffers: &[Vertex::LAYOUT, Instance::LAYOUT],
                    },
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleList,
                        strip_index_format: None,
                        front_face: wgpu::FrontFace::Ccw,
                        cull_mode: None,
                        unclipped_depth: false,
                        polygon_mode: wgpu::PolygonMode::Line,
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
                        module: &shared.shader,
                        entry_point: Some("fragment_main_wireframe"),
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

        commands
            .entity(entity)
            .insert(MeshRenderPipelinePerSurface {
                pipeline,
                wireframe_pipeline,
            });
    }
}

fn update_instance_buffer(
    wgpu: Res<WgpuContext>,
    instance_buffer: Option<ResMut<InstanceBuffer>>,
    meshes: Populated<(Entity, &GlobalTransform, Option<&mut InstanceId>), With<Mesh>>,
    mut commands: Commands,
    mut instance_data: Local<Vec<Instance>>,
) {
    // I always forget to clear this! keep this assert :3
    assert!(instance_data.is_empty());

    // create data for instance buffer
    for (entity, transform, instance_id) in meshes {
        let id = instance_data.len().try_into().unwrap();

        instance_data.push(Instance {
            model_matrix: transform.isometry().to_homogeneous(),
        });

        if let Some(mut instance_id) = instance_id {
            instance_id.0 = id;
        }
        else {
            commands.entity(entity).insert(InstanceId(id));
        }
    }

    let instance_data_bytes = bytemuck::cast_slice(&**instance_data);
    let instance_data_size = instance_data_bytes.len() as wgpu::BufferAddress;

    let create_and_fill_buffer = |allocate_size: u64| {
        let buffer = wgpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("mesh instance"),
            size: allocate_size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });

        let mut view = buffer.get_mapped_range_mut(..instance_data_size);
        view.copy_from_slice(instance_data_bytes);

        drop(view);
        buffer.unmap();

        InstanceBuffer {
            buffer,
            size: allocate_size,
        }
    };

    if let Some(mut instance_buffer) = instance_buffer {
        // instance buffer already exists. we'll try to reuse it

        if instance_buffer.size < instance_data_size {
            // new instance data is larger than already existing buffer. allocate a new one

            // double the previous size, but atleast enough for the instance data
            let allocate_size = instance_data_size.max(2 * instance_buffer.size);

            *instance_buffer = create_and_fill_buffer(allocate_size);
        }
        else {
            // new instance data fits into buffer. use staging pool to upload the data

            let mut command_encoder =
                wgpu.device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("upload mesh instance data"),
                    });

            let mut staging = WriteStagingTransaction::new(
                wgpu.staging_pool.belt(),
                &wgpu.device,
                &mut command_encoder,
            );

            staging.write_buffer_from_slice(
                instance_buffer.buffer.slice(..instance_data_size),
                instance_data_bytes,
            );

            staging.commit();
            wgpu.queue.submit([command_encoder.finish()]);
        }
    }
    else {
        // we don't have a instance buffer yet. create one.

        commands.insert_resource(create_and_fill_buffer(instance_data_size));
    }

    // don't forget!!!!111
    instance_data.clear();
}

fn render_meshes_with(
    atlas: Res<Atlas>,
    frames: Populated<(&mut Frame, &MeshRenderPipelinePerSurface, &AttachedCamera)>,
    camera_frustrums: Query<(&CameraFrustrum, &GlobalTransform)>,
    meshes: Populated<(&Mesh, &InstanceId, Option<&FrustrumCulled>)>,
    instance_buffer: Res<InstanceBuffer>,
    get_pipeline: impl Fn(&MeshRenderPipelinePerSurface) -> &wgpu::RenderPipeline,
) {
    let mut count_rendered = 0;
    let mut count_culled = 0;

    for (mut frame, pipeline, camera) in frames {
        let mut render_pass = frame.render_pass_mut();

        render_pass.set_pipeline(get_pipeline(pipeline));
        render_pass.set_bind_group(1, Some(atlas.bind_group()), &[]);
        render_pass.set_vertex_buffer(1, instance_buffer.buffer.slice(..));

        let frustrum_culling =
            camera_frustrums
                .get(camera.0)
                .ok()
                .map(|(camera_frustrum, camera_transform)| {
                    (camera_frustrum, camera_transform.isometry().inverse())
                });

        for (mesh, instance_id, cull_aabb) in &meshes {
            let cull = frustrum_culling.is_some_and(|(camera_frustrum, camera_transform_inv)| {
                cull_aabb.is_some_and(|cull_aabb| {
                    camera_frustrum.cull(&camera_transform_inv, &cull_aabb.aabb)
                })
            });

            if cull {
                count_culled += 1;
            }
            else {
                mesh.draw(&mut render_pass, 0, instance_id.0..(instance_id.0 + 1));
                count_rendered += 1;
            }
        }
    }

    tracing::trace!(count_rendered, count_culled, "rendered meshes");
}

fn render_meshes(
    atlas: Res<Atlas>,
    frames: Populated<(&mut Frame, &MeshRenderPipelinePerSurface, &AttachedCamera)>,
    camera_frustrums: Query<(&CameraFrustrum, &GlobalTransform)>,
    meshes: Populated<(&Mesh, &InstanceId, Option<&FrustrumCulled>)>,
    instance_buffer: Res<InstanceBuffer>,
) {
    render_meshes_with(
        atlas,
        frames,
        camera_frustrums,
        meshes,
        instance_buffer,
        |per_surface| &per_surface.pipeline,
    );
}

fn render_wireframes(
    atlas: Res<Atlas>,
    frames: Populated<(&mut Frame, &MeshRenderPipelinePerSurface, &AttachedCamera)>,
    camera_frustrums: Query<(&CameraFrustrum, &GlobalTransform)>,
    meshes: Populated<(&Mesh, &InstanceId, Option<&FrustrumCulled>)>,
    instance_buffer: Res<InstanceBuffer>,
) {
    render_meshes_with(
        atlas,
        frames,
        camera_frustrums,
        meshes,
        instance_buffer,
        |per_surface| &per_surface.wireframe_pipeline,
    );
}
