use std::ops::Range;

use bevy_ecs::{
    component::Component,
    entity::Entity,
    name::NameOrEntity,
    query::{
        Changed,
        Or,
        QueryData,
        Without,
    },
    resource::Resource,
    schedule::IntoScheduleConfigs,
    system::{
        Commands,
        Local,
        Populated,
        Res,
    },
};
use bytemuck::{
    Pod,
    Zeroable,
};
use nalgebra::Vector2;
use taffy::{
    AvailableSpace,
    Size,
};

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
        surface::Surface,
        text::{
            Font,
            FontBindGroup,
            FontBindGroupLayout,
            FontSystems,
            Text,
            TextSize,
        },
    },
    ui::{
        LeafMeasure,
        UiSystems,
    },
    wgpu::{
        WgpuContext,
        buffer::TypedArrayBuffer,
    },
};

#[derive(Clone, Copy, Debug, Default)]
pub struct TextLeafMeasure;

impl LeafMeasure for TextLeafMeasure {
    type Data = Res<'static, Font>;
    type Node = (&'static TextSize, &'static TextLayout);

    fn measure(
        &self,
        leaf: &<Self::Node as QueryData>::Item<'_, '_>,
        data: &<Self::Data as bevy_ecs::system::SystemParam>::Item<'_, '_>,
        known_dimensions: Size<Option<f32>>,
        available_space: Size<AvailableSpace>,
    ) -> Size<f32> {
        tracing::debug!(?known_dimensions, ?available_space);

        let (text_size, layout) = *leaf;
        let font = &**data;

        let displacement = font.glyph_displacement();
        let displacement = Vector2::new(
            text_size.height * displacement.x / displacement.y,
            text_size.height,
        );

        let width_constraint = known_dimensions.width.or(match available_space.width {
            AvailableSpace::MinContent | AvailableSpace::MaxContent => None,
            AvailableSpace::Definite(width) => Some(width),
        });
        let width_constraint = width_constraint
            .map(|width_constraint| (width_constraint / displacement.x).floor().max(1.0) as usize);

        let mut line_width = 0;
        let mut max_line_width = 0;
        let mut num_lines = 0;
        let mut is_first_chunk_on_line = true;

        for chunk in &layout.chunks {
            match chunk {
                TextLayoutChunk::Glyphs {
                    span: _,
                    num_glyphs,
                } => {
                    if !is_first_chunk_on_line
                        && width_constraint.is_some_and(|width_constraint| {
                            line_width + *num_glyphs > width_constraint
                        })
                    {
                        num_lines += 1;
                        line_width = *num_glyphs;
                    }
                    else {
                        line_width += *num_glyphs;
                    }

                    is_first_chunk_on_line = false;
                    max_line_width = max_line_width.max(line_width);
                }
                TextLayoutChunk::Spaces { num_spaces } => {
                    let num_spaces_on_this_line =
                        width_constraint.map_or(*num_spaces, |width_constraint| {
                            let min_spaces_on_this_line =
                                if is_first_chunk_on_line { 1 } else { 0 };

                            (width_constraint - line_width).max(min_spaces_on_this_line)
                        });

                    let num_spaces_on_next_line = *num_spaces - num_spaces_on_this_line;

                    line_width += num_spaces_on_this_line;

                    if num_spaces_on_next_line > 0 {
                        max_line_width = max_line_width.max(line_width);
                        num_lines += 1;
                        line_width = num_spaces_on_next_line;
                    }

                    is_first_chunk_on_line = false;
                }
                TextLayoutChunk::Newlines { num_newlines } => {
                    line_width = 0;
                    num_lines += *num_newlines;
                    is_first_chunk_on_line = true;
                }
            }
        }

        Size {
            width: max_line_width as f32 * displacement.x,
            height: num_lines as f32 * displacement.y,
        }
    }
}

pub(super) fn setup_text_systems(builder: &mut WorldBuilder) {
    builder
        .add_systems(
            schedule::Startup,
            create_pipeline_layout
                .in_set(RenderSystems::Setup)
                .after(FontSystems::Setup),
        )
        .add_systems(
            schedule::Render,
            (
                //text_layout.in_set(UiSystems::Layout),
                compute_text_layouts.before(UiSystems::Layout),
                create_pipeline.in_set(RenderSystems::BeginFrame),
                render_text.in_set(UiSystems::Render),
            ),
        );
}

fn create_pipeline_layout(
    wgpu: Res<WgpuContext>,
    frame_bind_group_layout: Res<FrameBindGroupLayout>,
    font_bind_group_layout: Res<FontBindGroupLayout>,
    mut commands: Commands,
) {
    let text_bind_group_layout =
        wgpu.device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("text rendering"),
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
            label: Some("text rendering"),
            bind_group_layouts: &[
                &frame_bind_group_layout.bind_group_layout,
                &font_bind_group_layout.bind_group_layout,
                &text_bind_group_layout,
            ],
            immediate_size: 0,
        });

    let shader = wgpu
        .device
        .create_shader_module(wgpu::include_wgsl!("text.wgsl"));

    commands.insert_resource(TextPipelineLayout {
        text_bind_group_layout,
        pipeline_layout,
        shader,
    });
}

fn create_pipeline(
    wgpu: Res<WgpuContext>,
    text_pipeline_layout: Res<TextPipelineLayout>,
    surfaces: Populated<(NameOrEntity, &Surface), Without<TextPipeline>>,
    mut commands: Commands,
) {
    for (entity, surface) in surfaces {
        tracing::debug!(surface = %entity, "creating text render pipeline for surface");

        let pipeline = wgpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("text rendering"),
                layout: Some(&text_pipeline_layout.pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &text_pipeline_layout.shader,
                    entry_point: Some("text_vertex"),
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
                    module: &text_pipeline_layout.shader,
                    entry_point: Some("text_fragment"),
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
            .insert(TextPipeline { pipeline });
    }
}

fn compute_text_layouts(
    font: Res<Font>,
    texts: Populated<
        (Entity, &Text, Option<&mut TextLayout>),
        Or<(Changed<Text>, Without<TextLayout>)>,
    >,
    mut commands: Commands,
    mut layout_run_buffer: Local<Vec<TextLayoutChunk>>,
) {
    for (entity, text, computed_text_layout) in texts {
        tracing::debug!(?entity, text = text.text, "layout text");

        assert!(layout_run_buffer.is_empty());

        let mut characters = text.text.char_indices().peekable();

        while let Some((start_index, character)) = characters.next() {
            match character {
                ' ' => {
                    if let Some(TextLayoutChunk::Spaces { num_spaces }) =
                        layout_run_buffer.last_mut()
                    {
                        *num_spaces += 1;
                    }
                    else {
                        layout_run_buffer.push(TextLayoutChunk::Spaces { num_spaces: 1 });
                    }
                }
                '\r' => {
                    // nop
                }
                '\n' => {
                    if let Some(TextLayoutChunk::Newlines { num_newlines }) =
                        layout_run_buffer.last_mut()
                    {
                        *num_newlines += 1;
                    }
                    else {
                        layout_run_buffer.push(TextLayoutChunk::Newlines { num_newlines: 1 });
                    }
                }
                _ => {
                    if let Some(_glyph_id) = font.glyph_id(character) {
                        let end_index = characters
                            .peek()
                            .map_or_else(|| text.text.len(), |(index, _)| *index);

                        if let Some(TextLayoutChunk::Glyphs { span, num_glyphs }) =
                            layout_run_buffer.last_mut()
                        {
                            span.end = end_index;
                            *num_glyphs += 1;
                        }
                        else {
                            layout_run_buffer.push(TextLayoutChunk::Glyphs {
                                span: start_index..end_index,
                                num_glyphs: 1,
                            });
                        }
                    }
                }
            }
        }

        if let Some(mut computed_text_layout) = computed_text_layout {
            computed_text_layout.chunks.clear();
            computed_text_layout
                .chunks
                .extend(layout_run_buffer.drain(..));
        }
        else {
            commands.entity(entity).insert(TextLayout {
                chunks: std::mem::take(&mut *layout_run_buffer),
            });
        }
    }
}

#[derive(Debug, Component)]
pub struct TextLayout {
    chunks: Vec<TextLayoutChunk>,
}

#[derive(Debug)]
enum TextLayoutChunk {
    Glyphs {
        span: Range<usize>,
        num_glyphs: usize,
    },
    Spaces {
        num_spaces: usize,
    },
    Newlines {
        num_newlines: usize,
    },
}

/*
/// System that performs text layout.
///
/// This will do the leaf measurement and update glyph buffers for texts
fn text_layout(
    font: Res<Font>,
    texts: Populated<(
        Entity,
        Ref<Text>,
        &mut LeafMeasure,
        Option<&mut GlyphBuffer>,
    )>,
    mut glyph_buffer_data: Local<Vec<GlyphData>>,
    wgpu: Res<WgpuContext>,
    text_pipeline_layout: Res<TextPipelineLayout>,
    mut staging: ResMut<Staging>,
    mut commands: Commands,
) {
    for (entity, text, mut leaf_measure, glyph_buffer) in texts {
        // todo
        let scaling = 2.0;

        assert!(glyph_buffer_data.is_empty());
        let mut update_glyph_buffer = false;

        leaf_measure.respond_with(
            |known_dimensions: Size<Option<f32>>, available_space: Size<AvailableSpace>| {
                if text.is_changed() {
                    tracing::debug!(?known_dimensions, ?available_space);

                    let width_constraint = known_dimensions.width.or(match available_space.width {
                        AvailableSpace::MinContent | AvailableSpace::MaxContent => None,
                        AvailableSpace::Definite(width) => Some(width),
                    });

                    // todo
                    //let _ = (known_dimensions, available_space);
                    //let width_constraint = None;

                    tracing::debug!(?entity, text = text.text, ?width_constraint, "layout text");

                    let mut offset = Vector2::zeros();
                    let displacement = font.glyph_displacement() * scaling;
                    let mut word_width = 0.0;
                    let mut is_first_word_on_line = true;
                    let mut max_width: f32 = 0.0;

                    for (_index, character) in text.text.char_indices() {
                        if character == '\n' {
                            // new line
                            offset.x = 0.0;
                            offset.y += displacement.y;
                            is_first_word_on_line = true;
                        }
                        else if character == ' ' {
                            // space

                            is_first_word_on_line = false;
                            offset.x += displacement.x;

                            if width_constraint
                                .is_some_and(|width_constraint| offset.x > width_constraint)
                            {
                                // if the space would overflow the width we just go to the next line
                                offset.x = 0.0;
                                offset.y += displacement.y;
                            }
                        }
                        else if let Some(glyph_id) = font.glyph_id(character) {
                            // text character

                            offset.x += displacement.x;
                            word_width += displacement.x;

                            if width_constraint
                                .is_some_and(|width_constraint| offset.x > width_constraint)
                                && !is_first_word_on_line
                            {
                                // move the whole word to the next line
                                offset.x = word_width;
                                offset.y += displacement.y;
                            }

                            glyph_buffer_data.push(GlyphData {
                                offset: offset,
                                glyph_id,
                                scaling,
                            })
                        }

                        max_width = max_width.max(offset.x);
                    }

                    update_glyph_buffer = true;

                    Some(Size {
                        width: max_width,
                        height: offset.y + displacement.y,
                    })
                }
                else {
                    None
                }
            },
        );

        if update_glyph_buffer {
            if glyph_buffer_data.is_empty() {
                commands.entity(entity).try_remove::<GlyphBuffer>();
            }
            else {
                if let Some(mut glyph_buffer) = glyph_buffer {
                    let glyph_buffer = &mut *glyph_buffer;

                    glyph_buffer.buffer.write_all(
                        &glyph_buffer_data,
                        |buffer| {
                            glyph_buffer.bind_group = create_glyph_buffer_bind_group(
                                &wgpu.device,
                                &text_pipeline_layout.text_bind_group_layout,
                                buffer,
                            );
                        },
                        &mut *staging,
                    );
                }
                else {
                    let buffer = TypedArrayBuffer::from_slice(
                        wgpu.device.clone(),
                        "text/glyphs",
                        wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
                        &glyph_buffer_data,
                    );

                    let bind_group = create_glyph_buffer_bind_group(
                        &wgpu.device,
                        &text_pipeline_layout.text_bind_group_layout,
                        buffer.buffer(),
                    );

                    commands.entity(entity).insert(GlyphBuffer {
                        buffer,
                        bind_group,
                        num_glyphs: glyph_buffer_data.len().try_into().unwrap(),
                    });
                }

                glyph_buffer_data.clear();
            }
        }
    }
} */

fn render_text(
    font: Res<FontBindGroup>,
    frames: Populated<(&mut Frame, &TextPipeline)>,
    texts: Populated<&GlyphBuffer>,
) {
    for (mut frame, text_pipeline) in frames {
        let render_pass = frame.render_pass_mut();

        render_pass.set_pipeline(&text_pipeline.pipeline);
        render_pass.set_bind_group(1, Some(&font.bind_group), &[]);

        for glyph_buffer in &texts {
            render_pass.set_bind_group(2, Some(&glyph_buffer.bind_group), &[]);

            let num_vertices = glyph_buffer.num_glyphs * 6;
            render_pass.draw(0..num_vertices, 0..1);
        }
    }
}

#[derive(Debug, Resource)]
struct TextPipelineLayout {
    text_bind_group_layout: wgpu::BindGroupLayout,
    shader: wgpu::ShaderModule,
    pipeline_layout: wgpu::PipelineLayout,
}

#[derive(Debug, Component)]
struct TextPipeline {
    pipeline: wgpu::RenderPipeline,
}

#[derive(Debug, Component)]
struct GlyphBuffer {
    buffer: TypedArrayBuffer<GlyphData>,
    bind_group: wgpu::BindGroup,
    num_glyphs: u32,
}

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
struct GlyphData {
    offset: Vector2<f32>,
    glyph_id: u32,
    scaling: f32,
}

fn create_glyph_buffer_bind_group(
    device: &wgpu::Device,
    bind_group_layout: &wgpu::BindGroupLayout,
    buffer: &wgpu::Buffer,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("text/glyphs"),
        layout: bind_group_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    })
}
