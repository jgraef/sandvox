use std::{
    marker::PhantomData,
    ops::Range,
};

use bevy_ecs::{
    component::Component,
    entity::Entity,
    name::NameOrEntity,
    query::{
        Added,
        Changed,
        Has,
        Or,
        ROQueryItem,
        With,
        Without,
    },
    resource::Resource,
    schedule::{
        IntoScheduleConfigs,
        common_conditions::any_match_filter,
    },
    system::{
        Commands,
        Local,
        Populated,
        Query,
        Res,
        ResMut,
        SystemParamItem,
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
    collide::Frustrum,
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
            CameraProjection,
            FrustrumCulled,
        },
        command::{
            AddRenderFunction,
            RenderFunction,
        },
        pass::{
            context::RenderPass,
            main_pass::{
                DepthPrepass,
                MainPass,
                MainPassLayout,
                MainPassPlugin,
                MainPassSystems,
            },
            phase,
        },
        render_target::RenderTarget,
        staging::Staging,
        surface::Surface,
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
        .require_plugin::<MainPassPlugin>()
        .init_resource::<RenderMeshStatistics>()
            .add_systems(
                schedule::Startup,
                (
                    create_mesh_pipeline_layout.in_set(RenderSystems::Setup).after(MainPassSystems::Prepare),
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

                ),
            )
            .add_render_function::<phase::Opaque, _>(RenderMeshes::<phase::Opaque>::default())
            .add_render_function::<phase::DepthPrepass, _>(RenderMeshes::<phase::DepthPrepass>::default())
            .add_render_function::<phase::Wireframe, _>(RenderMeshes::<phase::Wireframe>::default());
        Ok(())
    }
}

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

            let num_vertices = self.vertices.len();
            let num_indices = 3 * self.faces.len();

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
                num_indices,
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

    pub num_vertices: usize,
    pub num_indices: usize,

    pub bind_group: wgpu::BindGroup,
}

impl Mesh {
    pub fn byte_size(&self) -> usize {
        size_of::<Vertex>() * self.num_vertices + size_of::<u32>() * self.num_indices
    }
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
    opaque: wgpu::RenderPipeline,
    wireframe: wgpu::RenderPipeline,
    depth_prepass: Option<wgpu::RenderPipeline>,
}

#[profiling::function]
fn create_mesh_pipeline_layout(
    wgpu: Res<WgpuContext>,
    main_pass_layout: Res<MainPassLayout>,
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
                &main_pass_layout.bind_group_layout,
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
    surfaces: Populated<(NameOrEntity, &Surface)>,
    cameras: Populated<
        (NameOrEntity, &RenderTarget, Has<DepthPrepass>),
        (
            // todo: this should really check if there's *any* view that needs to render *anything*
            // opaque
            With<MainPass>,
            Without<MeshPipeline>,
        ),
    >,
    mut commands: Commands,
) {
    for (camera_entity, render_target, enable_depth_prepass) in cameras {
        if let Ok((surface_entity, surface)) = surfaces.get(render_target.0) {
            tracing::debug!(surface = %surface_entity, camera = %camera_entity, "creating mesh render pipeline for surface");

            let opaque = wgpu
                .device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("mesh/opaque"),
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
                        format: surface.depth_format(),
                        depth_write_enabled: !enable_depth_prepass,
                        depth_compare: if enable_depth_prepass {
                            wgpu::CompareFunction::Equal
                        }
                        else {
                            wgpu::CompareFunction::Less
                        },
                        stencil: Default::default(),
                        bias: Default::default(),
                    }),
                    multisample: Default::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &pipeline_layout.shader,
                        entry_point: Some("mesh_shaded_fragment"),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: surface.surface_format(),
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview_mask: None,
                    cache: None,
                });

            let wireframe = wgpu
                .device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("mesh/opaque"),
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
                        format: surface.depth_format(),
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
                            format: surface.surface_format(),
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview_mask: None,
                    cache: None,
                });

            let depth_prepass = enable_depth_prepass.then(|| {
                wgpu.device
                    .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: Some("mesh/depth-prepass"),
                        layout: Some(&pipeline_layout.layout),
                        vertex: wgpu::VertexState {
                            module: &pipeline_layout.shader,
                            entry_point: Some("mesh_depth_prepass_vertex"),
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
                            format: surface.depth_format(),
                            depth_write_enabled: true,
                            depth_compare: wgpu::CompareFunction::Less,
                            stencil: Default::default(),
                            bias: Default::default(),
                        }),
                        multisample: Default::default(),
                        fragment: None,
                        multiview_mask: None,
                        cache: None,
                    })
            });

            commands.entity(camera_entity.entity).insert(MeshPipeline {
                opaque,
                wireframe,
                depth_prepass,
            });
        }
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

struct RenderMeshes<P> {
    _marker: PhantomData<fn() -> P>,
}
impl<P> Default for RenderMeshes<P> {
    fn default() -> Self {
        Self {
            _marker: Default::default(),
        }
    }
}

trait RenderMeshesForPhase: Send + Sync + 'static {
    fn scope_label() -> &'static str;
    fn get_pipeline(pipeline: &MeshPipeline) -> &wgpu::RenderPipeline;
    fn vertices(num_indices: usize) -> Range<u32>;

    #[inline]
    fn count_stats(stats: &mut RenderMeshStatistics, culled: bool, num_vertices: usize) {
        let _ = (stats, culled, num_vertices);
    }

    #[inline]
    fn reset_stats(stats: &mut RenderMeshStatistics) {
        let _ = stats;
    }
}

impl RenderMeshesForPhase for phase::Opaque {
    #[inline]
    fn scope_label() -> &'static str {
        "mesh/opaque"
    }

    #[inline]
    fn get_pipeline(pipeline: &MeshPipeline) -> &wgpu::RenderPipeline {
        &pipeline.opaque
    }

    #[inline]
    fn vertices(num_indices: usize) -> Range<u32> {
        0..u32::try_from(num_indices).unwrap()
    }

    #[inline]
    fn count_stats(stats: &mut RenderMeshStatistics, culled: bool, num_vertices: usize) {
        if culled {
            stats.num_culled += 1;
        }
        else {
            stats.num_rendered += 1;
            stats.num_vertices += num_vertices;
        }
    }

    #[inline]
    fn reset_stats(stats: &mut RenderMeshStatistics) {
        *stats = Default::default();
    }
}

impl RenderMeshesForPhase for phase::Wireframe {
    #[inline]
    fn scope_label() -> &'static str {
        "mesh/wireframe"
    }

    #[inline]
    fn get_pipeline(pipeline: &MeshPipeline) -> &wgpu::RenderPipeline {
        &pipeline.wireframe
    }

    #[inline]
    fn vertices(num_indices: usize) -> Range<u32> {
        // n vertices are n/3 triangles, which require n lines to connect, which require
        // 2*n vertices
        0..u32::try_from(2 * num_indices).unwrap()
    }
}

impl RenderMeshesForPhase for phase::DepthPrepass {
    #[inline]
    fn scope_label() -> &'static str {
        "mesh/z-prepass"
    }

    #[inline]
    fn get_pipeline(pipeline: &MeshPipeline) -> &wgpu::RenderPipeline {
        pipeline
            .depth_prepass
            .as_ref()
            .expect("no depth-prepass pipeline")
    }

    #[inline]
    fn vertices(num_indices: usize) -> Range<u32> {
        0..u32::try_from(num_indices).unwrap()
    }
}

impl<P> RenderFunction for RenderMeshes<P>
where
    P: RenderMeshesForPhase,
{
    type Param = (
        Res<'static, InstanceBuffer>,
        ResMut<'static, RenderMeshStatistics>,
    );
    type ViewQuery = (
        &'static CameraProjection,
        &'static GlobalTransform,
        &'static MeshPipeline,
    );
    type ItemQuery = (
        &'static Mesh,
        &'static InstanceId,
        Option<&'static FrustrumCulled>,
    );

    #[profiling::function]
    fn prepare(&self, param: SystemParamItem<Self::Param>) {
        let (_instance_buffer, mut stats) = param;
        P::reset_stats(&mut stats);
    }

    #[profiling::function]
    fn render(
        &self,
        param: SystemParamItem<Self::Param>,
        render_pass: &mut RenderPass<'_>,
        view: ROQueryItem<Self::ViewQuery>,
        items: Query<Self::ItemQuery>,
    ) {
        let (instance_buffer, mut stats) = param;

        if let Some(instance_bind_group) = &instance_buffer.bind_group {
            let (camera_projection, camera_transform, pipeline) = view;

            let span = render_pass.enter_span(P::scope_label());

            render_pass.set_pipeline(P::get_pipeline(pipeline));
            render_pass.set_bind_group(1, instance_bind_group, &[]);

            let camera_frustrum = Frustrum {
                matrix: camera_projection.to_matrix()
                    * camera_transform.isometry.inverse().to_homogeneous(),
            };

            for (mesh, instance_id, cull_aabb) in &items {
                let cull = cull_aabb
                    .is_some_and(|cull_aabb| !camera_frustrum.intersect_aabb(&cull_aabb.aabb));

                if cull {
                    P::count_stats(&mut stats, true, mesh.num_indices);
                }
                else {
                    render_pass.set_bind_group(2, &mesh.bind_group, &[]);
                    render_pass.draw(
                        P::vertices(mesh.num_indices),
                        instance_id.0..(instance_id.0 + 1),
                    );

                    P::count_stats(&mut stats, false, mesh.num_indices);
                }
            }

            render_pass.exit_span(span);
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Resource)]
pub struct RenderMeshStatistics {
    pub num_rendered: usize,
    pub num_culled: usize,
    pub num_vertices: usize,
}
