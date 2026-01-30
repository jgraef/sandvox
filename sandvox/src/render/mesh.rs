use std::convert::identity;

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
    collide::frustrum::Frustrum,
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
            Camera,
            CameraProjection,
            FrustrumCulled,
        },
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
    wgpu::{
        WgpuContext,
        buffer::TypedArrayBuffer,
    },
};

#[derive(Clone, Copy, Debug)]
pub struct MeshPlugin;

impl Plugin for MeshPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder
            .add_systems(
                schedule::Startup,
                (
                    create_mesh_pipeline_layout.in_set(RenderSystems::Setup),
                    create_instance_buffer.in_set(RenderSystems::Setup),
                ),
            )
            .add_systems(
                schedule::Render,
                (
                    create_mesh_pipeline.in_set(RenderSystems::BeginFrame),
                    update_instance_buffer
                        .in_set(RenderSystems::BeginFrame)
                        .run_if(
                            any_match_filter::<(
                                With<Mesh>,
                                Or<(Changed<GlobalTransform>, Added<Mesh>)>,
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

    pub fn finish(
        &self,
        wgpu: &WgpuContext,
        label: &str,
        bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Option<Mesh> {
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
                    usage: wgpu::BufferUsages::STORAGE,
                });

            let index_buffer = wgpu
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(&format!("{label} indices")),
                    contents: bytemuck::cast_slice(&self.faces),
                    usage: wgpu::BufferUsages::STORAGE,
                });

            let num_vertices = 3 * u32::try_from(self.faces.len()).unwrap();

            let bind_group = wgpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(&format!("{label} bind group")),
                layout: bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: vertex_buffer.as_entire_binding(),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: index_buffer.as_entire_binding(),
                    },
                ],
            });

            Some(Mesh {
                vertex_buffer,
                index_buffer,
                num_vertices,
                bind_group,
            })
        }
    }
}

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
struct Instance {
    model_matrix: Matrix4<f32>,
}

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
pub struct Vertex {
    pub position: Vector4<f32>,
    pub normal: Vector4<f32>,
    pub uv: Point2<f32>,
    pub texture_id: u32,
    pub padding: u32,
}

#[derive(Clone, Debug, Component)]
pub struct Mesh {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,

    /// How many vertices are actually rendered (not how many are in the buffer)
    pub num_vertices: u32,

    pub bind_group: wgpu::BindGroup,
}

#[derive(Debug, Resource)]
struct InstanceBuffer {
    buffer: TypedArrayBuffer<Instance>,
    bind_group: Option<wgpu::BindGroup>,
}

#[derive(Clone, Copy, Debug, Component)]
struct InstanceId(u32);

#[derive(Debug, Resource)]
pub struct MeshPipelineLayout {
    layout: wgpu::PipelineLayout,
    shader: wgpu::ShaderModule,
    instance_bind_group_layout: wgpu::BindGroupLayout,
    pub mesh_bind_group_layout: wgpu::BindGroupLayout,
}

#[derive(Debug, Component)]
struct MeshPipeline {
    pipeline: wgpu::RenderPipeline,
    wireframe_pipeline: wgpu::RenderPipeline,
}

#[profiling::function]
fn create_mesh_pipeline_layout(
    wgpu: Res<WgpuContext>,
    frame_bind_group_layout: Res<FrameBindGroupLayout>,
    mut commands: Commands,
) {
    let instance_bind_group_layout =
        wgpu.device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("instance"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

    let mesh_bind_group_layout =
        wgpu.device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("mesh"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
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
            label: Some("mesh"),
            bind_group_layouts: &[
                &frame_bind_group_layout.bind_group_layout,
                &instance_bind_group_layout,
                &mesh_bind_group_layout,
            ],
            immediate_size: 0,
        });

    let shader = wgpu
        .device
        .create_shader_module(wgpu::include_wgsl!("mesh.wgsl"));

    commands.insert_resource(MeshPipelineLayout {
        layout,
        shader,
        instance_bind_group_layout,
        mesh_bind_group_layout,
    });
}

#[profiling::function]
fn create_mesh_pipeline(
    wgpu: Res<WgpuContext>,
    pipeline_layout: Res<MeshPipelineLayout>,
    surfaces: Populated<(Entity, NameOrEntity, &Surface), Without<MeshPipeline>>,
    mut commands: Commands,
) {
    for (entity, name, surface) in surfaces {
        tracing::trace!(surface = %name, "creating mesh render pipeline for surface");

        let pipeline = wgpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("mesh"),
                layout: Some(&pipeline_layout.layout),
                vertex: wgpu::VertexState {
                    module: &pipeline_layout.shader,
                    entry_point: Some("mesh_shaded_vertex"),
                    compilation_options: Default::default(),
                    buffers: &[],
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
                    module: &pipeline_layout.shader,
                    entry_point: Some("mesh_shaded_fragment"),
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
                    layout: Some(&pipeline_layout.layout),
                    vertex: wgpu::VertexState {
                        module: &pipeline_layout.shader,
                        entry_point: Some("mesh_wireframe_vertex"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::LineList,
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
                        entry_point: Some("mesh_wireframe_fragment"),
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

        commands.entity(entity).insert(MeshPipeline {
            pipeline,
            wireframe_pipeline,
        });
    }
}

#[profiling::function]
fn create_instance_buffer(wgpu: Res<WgpuContext>, mut commands: Commands) {
    let buffer = TypedArrayBuffer::new(
        wgpu.device.clone(),
        "mesh instance buffer",
        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
    );

    commands.insert_resource(InstanceBuffer {
        buffer,
        bind_group: None,
    });
}

#[profiling::function]
fn update_instance_buffer(
    wgpu: Res<WgpuContext>,
    layout: Res<MeshPipelineLayout>,
    mut instance_buffer: ResMut<InstanceBuffer>,
    meshes: Populated<(Entity, &GlobalTransform, Option<&mut InstanceId>), With<Mesh>>,
    mut commands: Commands,
    mut instance_data: Local<Vec<Instance>>,
    mut staging: ResMut<Staging>,
) {
    // I always forget to clear this! keep this assert :3
    assert!(instance_data.is_empty());

    // create data for instance buffer
    for (entity, transform, instance_id) in meshes {
        let id = instance_data.len().try_into().unwrap();

        instance_data.push(Instance {
            model_matrix: transform.isometry.to_homogeneous(),
        });

        if let Some(mut instance_id) = instance_id {
            instance_id.0 = id;
        }
        else {
            commands.entity(entity).insert(InstanceId(id));
        }
    }

    let instance_buffer = &mut *instance_buffer;
    instance_buffer.buffer.write_all(
        &instance_data,
        |buffer| {
            instance_buffer.bind_group =
                Some(wgpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("instance"),
                    layout: &layout.instance_bind_group_layout,
                    entries: &[wgpu::BindGroupEntry {
                        binding: 0,
                        resource: buffer.as_entire_binding(),
                    }],
                }));
        },
        &mut *staging,
    );

    // don't forget!!!!111
    instance_data.clear();
}

fn render_meshes_with(
    cameras: Populated<(&CameraProjection, &GlobalTransform, &RenderTarget), With<Camera>>,
    mut frames: Populated<(&mut Frame, &MeshPipeline)>,
    meshes: Populated<(&Mesh, &InstanceId, Option<&FrustrumCulled>)>,
    instance_buffer: Res<InstanceBuffer>,
    get_pipeline: impl Fn(&MeshPipeline) -> &wgpu::RenderPipeline,
    map_num_vertices: impl Fn(u32) -> u32,
    scope_label: &'static str,
) -> RenderMeshStatistics {
    let mut stats = RenderMeshStatistics::default();

    if let Some(instance_bind_group) = &instance_buffer.bind_group {
        for (camera_projection, camera_transform, render_target) in cameras {
            if let Ok((mut frame, pipeline)) = frames.get_mut(render_target.0) {
                let frame = frame.active_mut();
                let span = frame.enter_span(scope_label);

                frame.render_pass.set_pipeline(get_pipeline(pipeline));
                frame
                    .render_pass
                    .set_bind_group(1, instance_bind_group, &[]);

                let camera_frustrum = Frustrum {
                    matrix: camera_projection.to_matrix()
                        * camera_transform.isometry.inverse().to_homogeneous(),
                };

                for (mesh, instance_id, cull_aabb) in &meshes {
                    let cull = cull_aabb
                        .is_some_and(|cull_aabb| !camera_frustrum.intersect_aabb(&cull_aabb.aabb));

                    if cull {
                        stats.num_culled += 1;
                    }
                    else {
                        frame.render_pass.set_bind_group(2, &mesh.bind_group, &[]);
                        let num_vertices = map_num_vertices(mesh.num_vertices);
                        frame
                            .render_pass
                            .draw(0..num_vertices, instance_id.0..(instance_id.0 + 1));

                        stats.num_rendered += 1;
                        stats.num_vertices += num_vertices as usize;
                    }
                }

                frame.exit_span(span);
            }
        }
    }

    stats
}

#[profiling::function]
fn render_meshes(
    cameras: Populated<(&CameraProjection, &GlobalTransform, &RenderTarget), With<Camera>>,
    frames: Populated<(&mut Frame, &MeshPipeline)>,
    meshes: Populated<(&Mesh, &InstanceId, Option<&FrustrumCulled>)>,
    instance_buffer: Res<InstanceBuffer>,
    mut render_stats: ResMut<RenderMeshStatistics>,
) {
    *render_stats = render_meshes_with(
        cameras,
        frames,
        meshes,
        instance_buffer,
        |per_surface| &per_surface.pipeline,
        identity,
        "mesh-shaded",
    );
}

#[profiling::function]
fn render_wireframes(
    cameras: Populated<(&CameraProjection, &GlobalTransform, &RenderTarget), With<Camera>>,
    frames: Populated<(&mut Frame, &MeshPipeline)>,
    meshes: Populated<(&Mesh, &InstanceId, Option<&FrustrumCulled>)>,
    instance_buffer: Res<InstanceBuffer>,
) {
    render_meshes_with(
        cameras,
        frames,
        meshes,
        instance_buffer,
        |per_surface| &per_surface.wireframe_pipeline,
        |num_vertices| {
            // n vertices are n/3 triangles, which require n lines to connect, which require
            // 2*n vertices
            2 * num_vertices
        },
        "mesh-wireframe",
    );
}

#[derive(Clone, Copy, Debug, Default, Resource)]
pub struct RenderMeshStatistics {
    pub num_rendered: usize,
    pub num_culled: usize,
    pub num_vertices: usize,
}
