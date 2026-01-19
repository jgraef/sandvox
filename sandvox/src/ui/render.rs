use bevy_ecs::{
    component::Component,
    name::NameOrEntity,
    query::{
        Changed,
        Without,
    },
    resource::Resource,
    schedule::{
        IntoScheduleConfigs,
        SystemCondition,
        common_conditions::{
            any_component_removed,
            any_match_filter,
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
use nalgebra::Vector4;
use palette::LinSrgba;

use crate::{
    ecs::{
        plugin::WorldBuilder,
        schedule,
    },
    render::{
        RenderSystems,
        frame::{
            Frame,
            FrameBindGroupLayout,
        },
        staging::Staging,
        surface::Surface,
    },
    ui::{
        RoundedLayout,
        UiSystems,
    },
    wgpu::{
        WgpuContext,
        buffer::TypedArrayBuffer,
    },
};

pub(super) fn setup_render_systems(builder: &mut WorldBuilder) {
    builder
        .add_systems(
            schedule::Startup,
            (create_pipeline_layout, create_debug_mesh).in_set(RenderSystems::Setup),
        )
        .add_systems(
            schedule::Render,
            (
                create_pipeline.in_set(RenderSystems::BeginFrame),
                update_debug_mesh
                    .run_if(
                        any_component_removed::<RoundedLayout>
                            .or(any_match_filter::<Changed<RoundedLayout>>),
                    )
                    .before(render_debug_mesh),
                render_debug_mesh.in_set(UiSystems::Render),
            ),
        );
}

fn create_pipeline_layout(
    wgpu: Res<WgpuContext>,
    frame_bind_group_layout: Res<FrameBindGroupLayout>,
    mut commands: Commands,
) {
    let shader = wgpu
        .device
        .create_shader_module(wgpu::include_wgsl!("debug.wgsl"));

    let pipeline_layout = wgpu
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui/debug"),
            bind_group_layouts: &[&frame_bind_group_layout.bind_group_layout],
            immediate_size: 0,
        });

    commands.insert_resource(DebugPipelineLayout {
        shader,
        pipeline_layout,
    });
}

fn create_pipeline(
    wgpu: Res<WgpuContext>,
    debug_pipeline_layout: Res<DebugPipelineLayout>,
    surfaces: Populated<(NameOrEntity, &Surface), Without<DebugPipeline>>,
    mut commands: Commands,
) {
    for (entity, surface) in surfaces {
        let pipeline = wgpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("ui/debug"),
                layout: Some(&debug_pipeline_layout.pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &debug_pipeline_layout.shader,
                    entry_point: Some("debug_vertex"),
                    compilation_options: Default::default(),
                    buffers: &[DebugVertex::LAYOUT],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::LineStrip,
                    strip_index_format: Some(wgpu::IndexFormat::Uint32),
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None,
                    unclipped_depth: false,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    conservative: false,
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: surface.depth_texture_format(),
                    depth_write_enabled: false,
                    depth_compare: wgpu::CompareFunction::Always,
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &debug_pipeline_layout.shader,
                    entry_point: Some("debug_fragment"),
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

        commands
            .entity(entity.entity)
            .insert(DebugPipeline { pipeline });
    }
}

fn create_debug_mesh(wgpu: Res<WgpuContext>, mut commands: Commands) {
    commands.insert_resource(DebugMesh {
        vertex_buffer: TypedArrayBuffer::new(
            wgpu.device.clone(),
            "ui/debug/vertex",
            wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        ),
        index_buffer: TypedArrayBuffer::new(
            wgpu.device.clone(),
            "ui/debug/index",
            wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
        ),
    });
}

fn update_debug_mesh(
    mut debug_mesh: ResMut<DebugMesh>,
    rounded_layouts: Query<&RoundedLayout>,
    mut vertex_buffer_data: Local<Vec<DebugVertex>>,
    mut index_buffer_data: Local<Vec<u32>>,
    mut staging: ResMut<Staging>,
) {
    assert!(vertex_buffer_data.is_empty());
    assert!(index_buffer_data.is_empty());

    // todo
    let color = LinSrgba::new(1.0, 0.0, 0.0, 1.0);

    let mut make_vertex = |x, y| {
        vertex_buffer_data.push(DebugVertex {
            position: Vector4::new(x, y, 0.0, 1.0),
            color,
        })
    };

    let mut index = 0;
    for rounded_layout in rounded_layouts {
        let taffy::Point { x, y } = rounded_layout.location;
        let taffy::Size {
            width: w,
            height: h,
        } = rounded_layout.size;

        make_vertex(x, y);
        make_vertex(x + w, y);
        make_vertex(x + w, y + h);
        make_vertex(x, y + h);

        index_buffer_data.push(index);
        index_buffer_data.push(index + 1);
        index_buffer_data.push(index + 2);
        index_buffer_data.push(index + 3);
        index_buffer_data.push(index);
        index_buffer_data.push(u32::MAX);

        index += 4;
    }

    debug_mesh
        .vertex_buffer
        .write_all(&vertex_buffer_data, |_buffer| {}, &mut *staging);
    debug_mesh
        .index_buffer
        .write_all(&index_buffer_data, |_buffer| {}, &mut *staging);

    vertex_buffer_data.clear();
    index_buffer_data.clear();
}

fn render_debug_mesh(frames: Populated<(&mut Frame, &DebugPipeline)>, debug_mesh: Res<DebugMesh>) {
    if let (Some(vertex_buffer), Some(index_buffer)) = (
        debug_mesh.vertex_buffer.try_buffer(),
        debug_mesh.index_buffer.try_buffer(),
    ) {
        let num_indices = debug_mesh.index_buffer.len().try_into().unwrap();

        for (mut frame, text_pipeline) in frames {
            let render_pass = frame.render_pass_mut();

            render_pass.set_pipeline(&text_pipeline.pipeline);
            render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
            render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            render_pass.draw_indexed(0..num_indices, 0, 0..1);
        }
    }
}

#[derive(Debug, Resource)]
struct DebugPipelineLayout {
    shader: wgpu::ShaderModule,
    pipeline_layout: wgpu::PipelineLayout,
}

#[derive(Debug, Component)]
struct DebugPipeline {
    pipeline: wgpu::RenderPipeline,
}

// todo: should be one per surface (or ui root)
#[derive(Debug, Resource)]
struct DebugMesh {
    vertex_buffer: TypedArrayBuffer<DebugVertex>,
    index_buffer: TypedArrayBuffer<u32>,
}

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
struct DebugVertex {
    position: Vector4<f32>,
    color: LinSrgba<f32>,
}

impl DebugVertex {
    pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: size_of::<Self>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![
            0 => Float32x4,
            1 => Float32x4,
        ],
    };
}
