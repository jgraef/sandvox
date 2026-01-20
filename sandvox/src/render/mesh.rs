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
        WgpuContextBuilder,
        WgpuSystems,
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
                request_wgpu_features.in_set(WgpuSystems::RequestFeatures),
            )
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
    buffer: TypedArrayBuffer<Instance>,
}

#[derive(Clone, Copy, Debug, Component)]
struct InstanceId(u32);

#[derive(Debug, Resource)]
struct MeshPipelineLayout {
    layout: wgpu::PipelineLayout,
    shader: wgpu::ShaderModule,
}

#[derive(Debug, Component)]
struct MeshPipeline {
    pipeline: wgpu::RenderPipeline,
    wireframe_pipeline: wgpu::RenderPipeline,
}

fn create_mesh_pipeline_layout(
    wgpu: Res<WgpuContext>,
    frame_bind_group_layout: Res<FrameBindGroupLayout>,
    mut commands: Commands,
) {
    let layout = wgpu
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("mesh"),
            bind_group_layouts: &[&frame_bind_group_layout.bind_group_layout],
            immediate_size: 0,
        });

    let shader = wgpu
        .device
        .create_shader_module(wgpu::include_wgsl!("mesh.wgsl"));

    commands.insert_resource(MeshPipelineLayout { layout, shader });
}

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
                    module: &pipeline_layout.shader,
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
                    layout: Some(&pipeline_layout.layout),
                    vertex: wgpu::VertexState {
                        module: &pipeline_layout.shader,
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
                        module: &pipeline_layout.shader,
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

        commands.entity(entity).insert(MeshPipeline {
            pipeline,
            wireframe_pipeline,
        });
    }
}

fn create_instance_buffer(wgpu: Res<WgpuContext>, mut commands: Commands) {
    commands.insert_resource(InstanceBuffer {
        buffer: TypedArrayBuffer::new(
            wgpu.device.clone(),
            "mesh instance buffer",
            wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        ),
    });
}

fn update_instance_buffer(
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
            model_matrix: transform.isometry().to_homogeneous(),
        });

        if let Some(mut instance_id) = instance_id {
            instance_id.0 = id;
        }
        else {
            commands.entity(entity).insert(InstanceId(id));
        }
    }

    instance_buffer
        .buffer
        .write_all(&instance_data, |_buffer| {}, &mut *staging);

    // don't forget!!!!111
    instance_data.clear();
}

fn render_meshes_with(
    cameras: Populated<
        (Option<&CameraFrustrum>, &GlobalTransform, &RenderTarget),
        With<CameraProjection>,
    >,
    mut frames: Populated<(&mut Frame, &MeshPipeline)>,
    meshes: Populated<(&Mesh, &InstanceId, Option<&FrustrumCulled>)>,
    instance_buffer: Res<InstanceBuffer>,
    get_pipeline: impl Fn(&MeshPipeline) -> &wgpu::RenderPipeline,
) {
    if let Some(instance_buffer) = instance_buffer.buffer.try_buffer() {
        let mut count_rendered = 0;
        let mut count_culled = 0;

        for (camera_frustrum, camera_transform, render_target) in cameras {
            if let Ok((mut frame, pipeline)) = frames.get_mut(render_target.0) {
                let mut render_pass = frame.render_pass_mut();

                render_pass.set_pipeline(get_pipeline(pipeline));
                render_pass.set_vertex_buffer(1, instance_buffer.slice(..));

                let frustrum_culling = camera_frustrum.map(|camera_frustrum| {
                    (camera_frustrum, camera_transform.isometry().inverse())
                });

                for (mesh, instance_id, cull_aabb) in &meshes {
                    let cull =
                        frustrum_culling.is_some_and(|(camera_frustrum, camera_transform_inv)| {
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
        }

        tracing::trace!(count_rendered, count_culled, "rendered meshes");
    }
}

fn render_meshes(
    cameras: Populated<
        (Option<&CameraFrustrum>, &GlobalTransform, &RenderTarget),
        With<CameraProjection>,
    >,
    frames: Populated<(&mut Frame, &MeshPipeline)>,
    meshes: Populated<(&Mesh, &InstanceId, Option<&FrustrumCulled>)>,
    instance_buffer: Res<InstanceBuffer>,
) {
    render_meshes_with(cameras, frames, meshes, instance_buffer, |per_surface| {
        &per_surface.pipeline
    });
}

fn render_wireframes(
    cameras: Populated<
        (Option<&CameraFrustrum>, &GlobalTransform, &RenderTarget),
        With<CameraProjection>,
    >,
    frames: Populated<(&mut Frame, &MeshPipeline)>,
    meshes: Populated<(&Mesh, &InstanceId, Option<&FrustrumCulled>)>,
    instance_buffer: Res<InstanceBuffer>,
) {
    render_meshes_with(cameras, frames, meshes, instance_buffer, |per_surface| {
        &per_surface.wireframe_pipeline
    });
}
