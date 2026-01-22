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
    schedule::IntoScheduleConfigs,
    system::{
        Commands,
        Populated,
        Res,
        ResMut,
    },
};
use bytemuck::{
    Pod,
    Zeroable,
};
use nalgebra::{
    Point2,
    Vector2,
};

use crate::{
    ecs::{
        plugin::WorldBuilder,
        schedule,
    },
    render::{
        RenderSystems,
        atlas::AtlasId,
        frame::{
            Frame,
            FrameBindGroupLayout,
        },
        staging::Staging,
        surface::{
            RenderTarget,
            Surface,
        },
        text::GlyphId,
    },
    ui::{
        RedrawRequested,
        UiSystems,
        Viewport,
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
            create_pipeline_layout.in_set(RenderSystems::Setup),
        )
        .add_systems(
            schedule::Render,
            (
                /*update_render_buffers::<R>
                .run_if(
                    any_component_removed::<RoundedLayout>
                        .or(any_match_filter::<Changed<RoundedLayout>>),
                )
                .before(flush_render_buffers),*/
                (create_pipeline, create_render_buffer).in_set(RenderSystems::BeginFrame),
                (
                    flush_render_buffers.after(UiSystems::Render),
                    render_ui.after(flush_render_buffers),
                )
                    .in_set(RenderSystems::RenderUi),
                clear_render_requests.after(UiSystems::Render),
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
        .create_shader_module(wgpu::include_wgsl!("render.wgsl"));

    let bind_group_layout =
        wgpu.device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("ui"),
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

    let pipeline_layout = wgpu
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("ui"),
            bind_group_layouts: &[
                &frame_bind_group_layout.bind_group_layout,
                &bind_group_layout,
            ],
            immediate_size: 0,
        });

    commands.insert_resource(PipelineLayout {
        shader,
        bind_group_layout,
        pipeline_layout,
    });
}

fn create_pipeline(
    wgpu: Res<WgpuContext>,
    debug_pipeline_layout: Res<PipelineLayout>,
    surfaces: Populated<(Entity, &Surface), Without<Pipeline>>,
    mut commands: Commands,
) {
    for (entity, surface) in surfaces {
        let debug_pipeline = wgpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("ui/debug"),
                layout: Some(&debug_pipeline_layout.pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &debug_pipeline_layout.shader,
                    entry_point: Some("debug_vertex"),
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

        let quad_pipeline = wgpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("ui/quad"),
                layout: Some(&debug_pipeline_layout.pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &debug_pipeline_layout.shader,
                    entry_point: Some("quad_vertex"),
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
                    depth_compare: wgpu::CompareFunction::Always,
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &debug_pipeline_layout.shader,
                    entry_point: Some("quad_fragment"),
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
            debug_pipeline,
            quad_pipeline,
        });
    }
}

fn create_render_buffer(
    wgpu: Res<WgpuContext>,
    viewports: Populated<
        (NameOrEntity, &RenderTarget),
        Or<((Changed<RenderTarget>, With<Viewport>), Added<Viewport>)>,
    >,
    surfaces: Populated<NameOrEntity, Without<RenderBuffer>>,
    mut commands: Commands,
) {
    // todo: remove stale buffers (e.g. from a surface that is not a ui render
    // target anymore)

    for (viewport_entity, render_target) in viewports {
        if let Ok(surface_entity) = surfaces.get(render_target.0) {
            tracing::debug!(viewport = %viewport_entity, surface = %surface_entity, "creating render buffer");

            commands.entity(render_target.0).insert((
                RenderBuffer {
                    buffer: TypedArrayBuffer::new(
                        wgpu.device.clone(),
                        "ui/render",
                        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                    ),
                    bind_group: None,
                },
                RenderBufferBuilder::default(),
            ));
        }
    }
}

fn flush_render_buffers(
    wgpu: Res<WgpuContext>,
    pipeline_layout: Res<PipelineLayout>,
    render_buffers: Populated<
        (&mut RenderBuffer, &mut RenderBufferBuilder),
        Changed<RenderBufferBuilder>,
    >,
    mut staging: ResMut<Staging>,
) {
    tracing::trace!("flusing render buffers");

    for (mut render_buffer, mut render_buffer_builder) in render_buffers {
        // sort front to back
        // note: we don't think this even matters since we're rendering it as one mesh
        //render_buffer_builder
        //    .quads
        //    .sort_unstable_by_key(|quad| (-quad.layer, -quad.z));

        // upload buffer
        let render_buffer = &mut *render_buffer;
        render_buffer.buffer.write_all_with(
            render_buffer_builder.quads.len(),
            |view| {
                for (target, source) in view.iter_mut().zip(&render_buffer_builder.quads) {
                    *target = source.quad;
                }
            },
            |new_buffer| {
                render_buffer.bind_group =
                    Some(wgpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("ui"),
                        layout: &pipeline_layout.bind_group_layout,
                        entries: &[wgpu::BindGroupEntry {
                            binding: 0,
                            resource: new_buffer.as_entire_binding(),
                        }],
                    }));
            },
            &mut *staging,
        );

        // clear builder
        render_buffer_builder.quads.clear();

        // i know we just cleared it, but we're unlikely to remove and assert.
        assert!(render_buffer_builder.quads.is_empty());
    }
}

fn render_ui(
    surfaces: Populated<(&mut Frame, &Pipeline, &RenderBuffer)>,
    show_debug_outlines: Option<Res<ShowDebugOutlines>>,
) {
    for (mut frame, render_pipeline, render_buffer) in surfaces {
        let num_quads: u32 = render_buffer.buffer.len().try_into().unwrap();

        if let Some(bind_group) = &render_buffer.bind_group {
            let render_pass = frame.render_pass_mut();

            render_pass.set_bind_group(1, Some(bind_group), &[]);

            render_pass.set_pipeline(&render_pipeline.quad_pipeline);
            render_pass.draw(0..(6 * num_quads), 0..1);

            if show_debug_outlines.is_some() {
                render_pass.set_pipeline(&render_pipeline.debug_pipeline);
                render_pass.draw(0..(8 * num_quads), 0..1);
            }
        }
    }
}

fn clear_render_requests(
    requests: Populated<Entity, With<RedrawRequested>>,
    mut commands: Commands,
) {
    for entity in requests {
        tracing::trace!(?entity, "clearing render request");
        commands.entity(entity).try_remove::<RedrawRequested>();
    }
}

#[derive(Debug, Resource)]
struct PipelineLayout {
    shader: wgpu::ShaderModule,
    bind_group_layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
}

#[derive(Debug, Component)]
struct Pipeline {
    debug_pipeline: wgpu::RenderPipeline,
    quad_pipeline: wgpu::RenderPipeline,
}

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
struct Quad {
    position: Point2<f32>,
    size: Vector2<f32>,
    texture_id: u32,
    _padding: u32,
}

#[derive(Debug)]
struct LayeredQuad {
    layer: i16,
    z: i16,
    quad: Quad,
}

#[derive(Debug, Default, Component)]
pub struct RenderBufferBuilder {
    quads: Vec<LayeredQuad>,
}

impl RenderBufferBuilder {
    pub fn push_quad(&mut self, position: Point2<f32>, size: Vector2<f32>) -> QuadBuilder<'_> {
        let index = self.quads.len();
        self.quads.push(LayeredQuad {
            layer: 0,
            z: 0,
            quad: Quad {
                position,
                size,
                _padding: 0,
                texture_id: u32::MAX,
            },
        });

        QuadBuilder {
            quad: &mut self.quads[index],
        }
    }
}

#[derive(Debug)]
pub struct QuadBuilder<'a> {
    quad: &'a mut LayeredQuad,
}

impl<'a> QuadBuilder<'a> {
    pub fn set_layer(&mut self, layer: i16) -> &mut Self {
        self.quad.layer = layer;
        self
    }

    pub fn set_z(&mut self, z: i16) -> &mut Self {
        self.quad.z = z;
        self
    }

    pub fn set_atlas_texture(&mut self, atlas_id: AtlasId) -> &mut Self {
        self.quad.quad.texture_id = atlas_id.into();
        self
    }

    pub fn set_glyph_texture(&mut self, glyph_id: GlyphId) -> &mut Self {
        const GLYPH_BIT: u32 = 0x8000_0000;

        let mut glyph_id: u32 = glyph_id.into();
        assert!(glyph_id & GLYPH_BIT == 0);
        glyph_id |= GLYPH_BIT;

        self.quad.quad.texture_id = glyph_id;
        self
    }
}

#[derive(Debug, Component)]
struct RenderBuffer {
    buffer: TypedArrayBuffer<Quad>,

    // note: this lives in here for now. this is one bind group per surface/render pass, just like
    // the FrameBindGroup. So theoretically they can be merged. We think eventually we'd like to
    // have separate render-pass-global bindgroups for 3D and UI.
    bind_group: Option<wgpu::BindGroup>,
}

#[derive(Clone, Copy, Debug, Resource)]
pub struct ShowDebugOutlines;
