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
        ResMut,
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
        staging::Staging,
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
        LayoutCache,
        LeafMeasure,
        RoundedLayout,
        UiSystems,
    },
    wgpu::{
        WgpuContext,
        buffer::TypedArrayBuffer,
    },
};

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
                create_pipeline.in_set(RenderSystems::BeginFrame),
                compute_text_layouts.before(UiSystems::Layout),
                update_glyph_buffers
                    .after(compute_text_layouts)
                    .before(render_text),
                render_text.in_set(UiSystems::Render),
            ),
        );
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TextLeafMeasure;

impl LeafMeasure for TextLeafMeasure {
    type Data = Res<'static, Font>;
    type Node = (Option<&'static TextSize>, &'static TextBuffer);

    fn measure(
        &self,
        (text_size, text_buffer): &mut <Self::Node as QueryData>::Item<'_, '_>,
        font: &mut <Self::Data as bevy_ecs::system::SystemParam>::Item<'_, '_>,
        known_dimensions: Size<Option<f32>>,
        available_space: Size<AvailableSpace>,
    ) -> Size<f32> {
        // this is basically the size of a glyph.
        //
        // technically glyphs are all different sizes (their minimal bounding boxes),
        // but since we are using a monospace font each glyph will move the cursor the
        // same amount.
        //
        // this is needed to calculate the width constraint (in "characters") and at the
        // end when we return the measure
        let displacement =
            font.glyph_displacement() * text_size.copied().unwrap_or_default().scaling;

        // calculate width constraint in number of "characters"
        let width_constraint = known_dimensions.width.or(match available_space.width {
            AvailableSpace::MinContent | AvailableSpace::MaxContent => None,
            AvailableSpace::Definite(width) => Some(width),
        });
        let width_constraint = width_constraint
            .map(|width_constraint| (width_constraint / displacement.x).floor().max(1.0) as usize);

        let size = text_buffer.calculate_positions(width_constraint).fold(
            Vector2::<usize>::zeros(),
            |mut accu, positioned| {
                match positioned {
                    PositionedTextChunk::Glyphs {
                        span: _,
                        offset,
                        num_glyphs,
                    } => {
                        accu.x = accu.x.max(offset.x + num_glyphs);
                        accu.y = accu.y.max(offset.y + 1);
                    }
                    PositionedTextChunk::Spaces { offset, num_spaces } => {
                        accu.x = accu.x.max(offset.x + num_spaces);
                        accu.y = accu.y.max(offset.y + 1);
                    }
                }
                accu
            },
        );

        let size = size.cast::<f32>().component_mul(&displacement);

        Size {
            width: size.x,
            height: size.y,
        }
    }
}

/// System that creates the pipeline layout for text rendering
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
        tracing::trace!(surface = %entity, "creating text render pipeline for surface");

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

/// System that calculates text layouts
///
/// The layout doesn't contain positions for the glyphs yet. This is done when
/// the text measure function runs.
fn compute_text_layouts(
    font: Res<Font>,
    texts: Populated<
        (Entity, &Text, Option<&mut TextBuffer>, &mut LayoutCache),
        Or<(Changed<Text>, Without<TextBuffer>)>,
    >,
    mut commands: Commands,
    mut layout_run_buffer: Local<Vec<TextBufferChunk>>,
) {
    for (entity, text, computed_text_layout, mut layout_cache) in texts {
        tracing::trace!(?entity, text = text.text, "layout text");

        assert!(layout_run_buffer.is_empty());

        let mut characters = text.text.char_indices().peekable();

        while let Some((start_index, character)) = characters.next() {
            match character {
                ' ' => {
                    if let Some(TextBufferChunk::Spaces { num_spaces }) =
                        layout_run_buffer.last_mut()
                    {
                        *num_spaces += 1;
                    }
                    else {
                        layout_run_buffer.push(TextBufferChunk::Spaces { num_spaces: 1 });
                    }
                }
                '\r' => {
                    // nop
                }
                '\n' => {
                    if let Some(TextBufferChunk::Newlines { num_newlines }) =
                        layout_run_buffer.last_mut()
                    {
                        *num_newlines += 1;
                    }
                    else {
                        layout_run_buffer.push(TextBufferChunk::Newlines { num_newlines: 1 });
                    }
                }
                _ => {
                    if let Some(_glyph_id) = font.glyph_id(character) {
                        let end_index = characters
                            .peek()
                            .map_or_else(|| text.text.len(), |(index, _)| *index);

                        if let Some(TextBufferChunk::Glyphs { span, num_glyphs }) =
                            layout_run_buffer.last_mut()
                        {
                            span.end = end_index;
                            *num_glyphs += 1;
                        }
                        else {
                            layout_run_buffer.push(TextBufferChunk::Glyphs {
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
            commands.entity(entity).insert(TextBuffer {
                chunks: std::mem::take(&mut *layout_run_buffer),
            });
        }

        // clear tree layout cache
        layout_cache.clear();
    }
}

#[derive(Debug, Component)]
pub struct TextBuffer {
    chunks: Vec<TextBufferChunk>,
}

impl TextBuffer {
    fn calculate_positions(
        &self,
        width_constraint: Option<usize>,
    ) -> impl Iterator<Item = PositionedTextChunk> {
        PositionedTextChunks {
            chunks: self.chunks.iter(),
            width_constraint,
            cursor: Vector2::zeros(),
            buffered_spaces: 0,
        }
    }
}

#[derive(Clone, Debug)]
enum TextBufferChunk {
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

struct PositionedTextChunks<'a> {
    chunks: std::slice::Iter<'a, TextBufferChunk>,
    width_constraint: Option<usize>,
    cursor: Vector2<usize>,
    buffered_spaces: usize,
}

impl<'a> Iterator for PositionedTextChunks<'a> {
    type Item = PositionedTextChunk;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            while self.buffered_spaces > 0 {
                let mut num_spaces =
                    self.width_constraint
                        .map_or(self.buffered_spaces, |width_constraint| {
                            width_constraint
                                .saturating_sub(self.cursor.x)
                                .min(self.buffered_spaces)
                        });

                if num_spaces == 0 {
                    if self.cursor.x == 0 {
                        num_spaces = 1;
                    }
                    else {
                        self.cursor.y += 1;
                        self.cursor.x = 0;
                        continue;
                    }
                }

                self.buffered_spaces -= num_spaces;
                let positioned = PositionedTextChunk::Spaces {
                    offset: self.cursor,
                    num_spaces,
                };

                self.cursor.x += num_spaces;

                return Some(positioned);
            }

            match self.chunks.next()? {
                TextBufferChunk::Glyphs { span, num_glyphs } => {
                    // a span of glyphs that are always on the same line

                    if self.cursor.x > 0
                        && self.width_constraint.is_some_and(|width_constraint| {
                            self.cursor.x + *num_glyphs > width_constraint
                        })
                    {
                        // this bit of text would overflow the line and we can move it to the
                        // next line (we don't move it to the next
                        // line if it's the first chunk of text on a
                        // line)
                        self.cursor.y += 1;
                        self.cursor.x = 0;
                    }

                    let positioned = PositionedTextChunk::Glyphs {
                        span: span.clone(),
                        offset: self.cursor,
                        num_glyphs: *num_glyphs,
                    };

                    self.cursor.x += *num_glyphs;

                    return Some(positioned);
                }
                TextBufferChunk::Spaces { num_spaces } => {
                    // a bunch of spaces. they can be split whever.

                    self.buffered_spaces = *num_spaces;
                }
                TextBufferChunk::Newlines { num_newlines } => {
                    // new lines

                    self.cursor.x = 0;
                    self.cursor.y += *num_newlines;
                }
            }
        }
    }
}

#[derive(Clone, Debug)]
enum PositionedTextChunk {
    Glyphs {
        span: Range<usize>,
        offset: Vector2<usize>,
        num_glyphs: usize,
    },
    Spaces {
        offset: Vector2<usize>,
        num_spaces: usize,
    },
}

fn update_glyph_buffers(
    font: Res<Font>,
    texts: Populated<
        (
            Entity,
            &Text,
            &TextBuffer,
            &TextSize,
            &RoundedLayout,
            Option<&mut GlyphBuffer>,
        ),
        Or<(
            Changed<TextBuffer>,
            Changed<RoundedLayout>,
            Without<GlyphBuffer>,
        )>,
    >,
    mut glyph_buffer_data: Local<Vec<GlyphData>>,
    wgpu: Res<WgpuContext>,
    text_pipeline_layout: Res<TextPipelineLayout>,
    mut staging: ResMut<Staging>,
    mut commands: Commands,
) {
    let displacement = font.glyph_displacement();

    for (entity, text, text_buffer, text_size, rounded_layout, glyph_buffer) in texts {
        assert!(glyph_buffer_data.is_empty());

        let content_offset = Vector2::new(
            rounded_layout.content_box_x(),
            rounded_layout.content_box_y(),
        );
        let content_size = Vector2::new(
            rounded_layout.content_box_width(),
            rounded_layout.content_box_height(),
        );

        let displacement = displacement * text_size.scaling;
        let width_constraint = (content_size.x / displacement.x).floor().max(0.0) as usize;

        tracing::trace!(?entity, text = ?text.text, ?content_offset, ?content_size, "update glyph buffer");

        for positioned in text_buffer.calculate_positions(Some(width_constraint)) {
            match positioned {
                PositionedTextChunk::Glyphs {
                    span,
                    offset,
                    num_glyphs: _,
                } => {
                    let mut offset =
                        offset.cast::<f32>().component_mul(&displacement) + content_offset;

                    for character in text.text[span.clone()].chars() {
                        if let Some(glyph_id) = font.glyph_id_or_replacement(character) {
                            glyph_buffer_data.push(GlyphData {
                                offset,
                                glyph_id,
                                scaling: text_size.scaling,
                            });

                            offset.x += displacement.x;
                        }
                    }
                }
                PositionedTextChunk::Spaces {
                    offset: _,
                    num_spaces: _,
                } => {
                    // nop
                }
            }
        }

        // write glyph buffer data
        let num_glyphs = glyph_buffer_data.len().try_into().unwrap();
        if num_glyphs == 0 {
            if glyph_buffer.is_some() {
                commands.entity(entity).try_remove::<GlyphBuffer>();
            }
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
                glyph_buffer.num_glyphs = num_glyphs;
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
                    num_glyphs,
                });
            }

            glyph_buffer_data.clear();
        }
    }
}

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
