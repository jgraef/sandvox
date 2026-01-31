use std::ops::Range;

use bevy_ecs::{
    change_detection::DetectChangesMut,
    component::Component,
    name::NameOrEntity,
    query::{
        Changed,
        ROQueryItem,
        With,
        Without,
    },
    resource::Resource,
    schedule::IntoScheduleConfigs,
    system::{
        Commands,
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
use itertools::Itertools;
use nalgebra::{
    Point2,
    Vector2,
};
use palette::{
    LinSrgba,
    Srgba,
};

use crate::{
    ecs::{
        plugin::WorldBuilder,
        schedule,
    },
    render::{
        RenderSystems,
        atlas::AtlasHandle,
        command::{
            AddRenderFunction,
            RenderFunction,
        },
        pass::{
            RenderPass,
            phase,
            ui_pass::{
                UiPass,
                UiPassLayout,
            },
        },
        render_target::RenderTarget,
        staging::Staging,
        surface::Surface,
        text::GlyphId,
    },
    ui::{
        UiSystems,
        view::View,
    },
    wgpu::{
        WgpuContext,
        buffer::TypedArrayBuffer,
    },
};

#[profiling::function]
pub(super) fn setup_render_systems(builder: &mut WorldBuilder) {
    builder
        .add_systems(
            schedule::Startup,
            create_layout.in_set(RenderSystems::Setup),
        )
        .add_systems(
            schedule::Render,
            (
                create_render_buffer.in_set(RenderSystems::BeginFrame),
                (create_pipeline, flush_render_buffers).before(RenderSystems::RenderUi),
                clear_render_requests.after(UiSystems::Render),
            ),
        )
        .add_render_function::<phase::Ui, _>(RenderUi);
}

#[profiling::function]
fn create_layout(
    wgpu: Res<WgpuContext>,
    ui_pass_layout: Res<UiPassLayout>,
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
            bind_group_layouts: &[&ui_pass_layout.bind_group_layout, &bind_group_layout],
            immediate_size: 0,
        });

    commands.insert_resource(UiLayout {
        shader,
        bind_group_layout,
        pipeline_layout,
    });
}

#[profiling::function]
fn create_pipeline(
    wgpu: Res<WgpuContext>,
    debug_pipeline_layout: Res<UiLayout>,
    surfaces: Populated<(NameOrEntity, &Surface)>,
    views: Populated<
        (NameOrEntity, &RenderTarget),
        (
            // todo: this should really check if there's *any* view that needs to render *anything*
            // opaque
            With<UiPass>,
            With<View>,
            Without<UiPipeline>,
        ),
    >,
    mut commands: Commands,
) {
    for (view_entity, render_target) in views {
        if let Ok((surface_entity, surface)) = surfaces.get(render_target.0) {
            tracing::debug!(surface = %surface_entity, view = %view_entity, "creating ui render pipeline for surface");
            let debug_pipeline =
                wgpu.device
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
                        depth_stencil: None,
                        multisample: Default::default(),
                        fragment: Some(wgpu::FragmentState {
                            module: &debug_pipeline_layout.shader,
                            entry_point: Some("debug_fragment"),
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

            let quad_pipeline =
                wgpu.device
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
                        depth_stencil: None,
                        multisample: Default::default(),
                        fragment: Some(wgpu::FragmentState {
                            module: &debug_pipeline_layout.shader,
                            entry_point: Some("quad_fragment"),
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

            commands.entity(view_entity.entity).insert(UiPipeline {
                debug_pipeline,
                quad_pipeline,
            });
        }
    }
}

#[profiling::function]
fn create_render_buffer(
    wgpu: Res<WgpuContext>,
    views: Populated<NameOrEntity, (With<UiPass>, Without<RenderBuffer>)>,
    mut commands: Commands,
) {
    // todo: remove stale buffers (e.g. from a surface that is not a ui render
    // target anymore)

    for view_entity in views {
        tracing::debug!(viewport = %view_entity, "creating render buffer");

        commands.entity(view_entity.entity).insert((
            RenderBuffer {
                buffer: TypedArrayBuffer::new(
                    wgpu.device.clone(),
                    "ui/render",
                    wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                ),
                bind_group: None,
                layers: vec![],
            },
            RenderBufferBuilder::default(),
        ));
    }
}

#[profiling::function]
fn flush_render_buffers(
    wgpu: Res<WgpuContext>,
    pipeline_layout: Res<UiLayout>,
    render_buffers: Populated<
        (&mut RenderBuffer, &mut RenderBufferBuilder),
        Changed<RenderBufferBuilder>,
    >,
    mut staging: ResMut<Staging>,
) {
    tracing::trace!("flusing render buffers");

    for (mut render_buffer, mut render_buffer_builder) in render_buffers {
        // sort quads by order
        render_buffer_builder.sort();

        // determine layers
        render_buffer.layers.clear();
        render_buffer.layers.extend(render_buffer_builder.layers());

        // upload buffer
        let render_buffer = &mut *render_buffer;
        render_buffer.buffer.write_all_with(
            render_buffer_builder.quads.len(),
            |view| {
                for (target, source) in view.iter_mut().zip(&render_buffer_builder.quads) {
                    *target = *source;
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

        // we had a bug that write_all_with didn't update the length
        assert_eq!(
            render_buffer.buffer.len(),
            render_buffer_builder.quads.len()
        );

        // clear builder
        render_buffer_builder.clear();

        // i know we just cleared it, but we're unlikely to remove and assert.
        assert!(render_buffer_builder.quads.is_empty());
    }
}

#[derive(Debug)]
struct RenderUi;

impl RenderFunction for RenderUi {
    type Param = (Option<Res<'static, ShowDebugOutlines>>,);
    type ViewQuery = (&'static UiPipeline, &'static RenderBuffer);
    type ItemQuery = ();

    #[profiling::function]
    fn render(
        &self,
        param: SystemParamItem<Self::Param>,
        render_pass: &mut RenderPass<'_>,
        view: ROQueryItem<Self::ViewQuery>,
        items: Query<Self::ItemQuery>,
    ) {
        let show_debug_outlines = param.0.is_some();
        let (pipeline, render_buffer) = view;
        let _ = items;

        if let Some(bind_group) = &render_buffer.bind_group {
            let span = render_pass.enter_span("ui");

            // bind bind group containing the render buffer
            render_pass.set_bind_group(1, Some(bind_group), &[]);

            // draw render buffer (textured quads)
            render_pass.set_pipeline(&pipeline.quad_pipeline);
            for layer in &render_buffer.layers {
                let start = layer.start * 6;
                let end = layer.end * 6;
                render_pass.draw(start..end, 0..1);
            }

            // draw debug outlines for render buffer
            if show_debug_outlines {
                let num_quads: u32 = render_buffer.buffer.len().try_into().unwrap();

                render_pass.set_pipeline(&pipeline.debug_pipeline);
                render_pass.draw(0..(8 * num_quads), 0..1);
            }

            render_pass.exit_span(span);
        }
    }
}

#[profiling::function]
fn clear_render_requests(requests: Populated<(NameOrEntity, &mut View), Changed<View>>) {
    for (entity, mut view) in requests {
        if view.render {
            tracing::trace!(%entity, "clearing render request");
            view.bypass_change_detection().render = false;
        }
    }
}

#[derive(Debug, Resource)]
struct UiLayout {
    shader: wgpu::ShaderModule,
    bind_group_layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
}

#[derive(Debug, Component)]
struct UiPipeline {
    debug_pipeline: wgpu::RenderPipeline,
    quad_pipeline: wgpu::RenderPipeline,
}

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
struct Quad {
    position: Point2<f32>,
    size: Vector2<f32>,
    texture_id: u32,
    order: u32,
    _padding: [u32; 2],
    tint: LinSrgba<f32>,
}

#[derive(Debug, Default, Component)]
pub struct RenderBufferBuilder {
    quads: Vec<Quad>,
    max_order: u32,
}

impl RenderBufferBuilder {
    pub fn push_quad(
        &mut self,
        position: Point2<f32>,
        size: Vector2<f32>,
        order: u32,
        tint: Option<Srgba<f32>>,
    ) -> QuadBuilder<'_> {
        let index = self.quads.len();

        self.quads.push(Quad {
            position,
            size,
            texture_id: u32::MAX,
            order,
            _padding: Default::default(),
            tint: tint.map_or_else(
                || LinSrgba::new(0.0, 0.0, 0.0, 1.0),
                |tint| tint.into_linear(),
            ),
        });

        self.max_order = self.max_order.max(order);

        QuadBuilder {
            quad: &mut self.quads[index],
        }
    }

    fn clear(&mut self) {
        self.quads.clear();
        self.max_order = 0;
    }

    fn sort(&mut self) {
        self.quads.sort_unstable_by_key(|quad| quad.order);
    }

    fn layers(&self) -> impl Iterator<Item = Range<u32>> {
        #[derive(Debug)]
        struct Layer {
            first: u32,
            last: u32,
            order: u32,
        }

        self.quads
            .iter()
            .enumerate()
            .map(|(i, quad)| {
                let i = u32::try_from(i).unwrap();
                Layer {
                    first: i,
                    last: i,
                    order: quad.order,
                }
            })
            .coalesce(|previous, current| {
                if previous.order == current.order {
                    Ok(Layer {
                        first: previous.first,
                        last: current.last,
                        order: current.order,
                    })
                }
                else {
                    Err((previous, current))
                }
            })
            .map(|layer| layer.first..(layer.last + 1))
    }
}

#[derive(Debug)]
pub struct QuadBuilder<'a> {
    quad: &'a mut Quad,
}

impl<'a> QuadBuilder<'a> {
    pub fn set_atlas_texture(&mut self, atlas_handle: &AtlasHandle) -> &mut Self {
        self.quad.texture_id = atlas_handle.id();
        self
    }

    pub fn set_glyph_texture(&mut self, glyph_id: GlyphId) -> &mut Self {
        const GLYPH_BIT: u32 = 0x8000_0000;

        let mut glyph_id: u32 = glyph_id.into();
        assert!(glyph_id & GLYPH_BIT == 0);
        glyph_id |= GLYPH_BIT;

        self.quad.texture_id = glyph_id;
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

    layers: Vec<Range<u32>>,
}

#[derive(Clone, Copy, Debug, Resource)]
pub struct ShowDebugOutlines;
