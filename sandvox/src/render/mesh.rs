use std::ops::Range;

use bevy_ecs::{
    component::Component,
    entity::Entity,
    name::NameOrEntity,
    query::Without,
    resource::Resource,
    schedule::IntoScheduleConfigs,
    system::{
        Commands,
        Populated,
        Res,
    },
};
use bytemuck::{
    Pod,
    Zeroable,
};
use color_eyre::eyre::Error;
use nalgebra::{
    Point2,
    Point3,
    Vector2,
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
        camera::CameraBindGroupLayout,
        frame::Frame,
        surface::Surface,
        texture_atlas::{
            Atlas,
            AtlasSystems,
        },
    },
    voxel::block_face::BlockFace,
    wgpu::WgpuContext,
};

#[derive(Clone, Copy, Debug)]
pub struct MeshPlugin;

impl Plugin for MeshPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder
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
                    render_chunks.after(RenderSystems::BeginFrame),
                ),
            );
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

    pub fn push_block_face(
        &mut self,
        anchor: Point3<u16>,
        size: Vector2<u16>,
        face: BlockFace,
        texture_id: u32,
    ) {
        let normal = face.normal().cast::<f32>().to_homogeneous();
        let positions = face.vertices(size);
        let uvs = face.uvs(size);
        let faces = face.faces();

        let vertices = (0..4).map(|i| {
            Vertex {
                position: (anchor + positions[i].coords)
                    .cast::<f32>()
                    .to_homogeneous(),
                normal,
                uv: uvs[i].coords.cast::<f32>().into(),
                texture_id,
            }
        });

        let faces = faces.map(|face| face.map(u32::from));

        self.push(vertices, faces);
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
pub struct Vertex {
    position: Vector4<f32>,
    normal: Vector4<f32>,
    uv: Point2<f32>,
    texture_id: u32,
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
struct MeshRenderPipelineShared {
    layout: wgpu::PipelineLayout,
    shader: wgpu::ShaderModule,
}

impl MeshRenderPipelineShared {
    fn new(
        wgpu: &WgpuContext,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        atlas_bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let layout = wgpu
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("mesh"),
                bind_group_layouts: &[&camera_bind_group_layout, &atlas_bind_group_layout],
                immediate_size: 0,
            });

        let shader = wgpu
            .device
            .create_shader_module(wgpu::include_wgsl!("mesh.wgsl"));

        Self { layout, shader }
    }
}

#[derive(Debug, Component)]
struct MeshRenderPipelinePerSurface {
    pipeline: wgpu::RenderPipeline,
}

impl MeshRenderPipelinePerSurface {
    fn new(wgpu: &WgpuContext, shared: &MeshRenderPipelineShared, surface: &Surface) -> Self {
        let pipeline = wgpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("mesh"),
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

fn create_mesh_render_pipeline_shared(
    wgpu: Res<WgpuContext>,
    camera_bind_group_layout: Res<CameraBindGroupLayout>,
    atlas: Res<Atlas>,
    mut commands: Commands,
) {
    commands.insert_resource(MeshRenderPipelineShared::new(
        &wgpu,
        &camera_bind_group_layout.bind_group_layout,
        atlas.bind_group_layout(),
    ));
}

fn create_mesh_render_pipeline_for_surfaces(
    wgpu: Res<WgpuContext>,
    shared: Res<MeshRenderPipelineShared>,
    surfaces: Populated<(Entity, NameOrEntity, &Surface), Without<MeshRenderPipelinePerSurface>>,
    mut commands: Commands,
) {
    for (entity, name, surface) in surfaces {
        tracing::trace!(surface = %name, "creating mesh render pipeline for surface");

        commands
            .entity(entity)
            .insert(MeshRenderPipelinePerSurface::new(&wgpu, &shared, surface));
    }
}

fn render_chunks(
    atlas: Res<Atlas>,
    frames: Populated<(&mut Frame, &MeshRenderPipelinePerSurface)>,
    chunk_meshes: Populated<(&Mesh, &GlobalTransform)>,
) {
    for (mut frame, pipeline) in frames {
        let mut render_pass = frame.render_pass_mut();

        render_pass.set_pipeline(&pipeline.pipeline);
        render_pass.set_bind_group(1, Some(atlas.bind_group()), &[]);

        let mut count = 0;
        for (mesh, transform) in &chunk_meshes {
            // todo: bind transform
            let _ = transform;

            mesh.draw(&mut render_pass, 0, 0..1);
            count += 1;
        }

        tracing::trace!("rendered {count} meshes");
    }
}
