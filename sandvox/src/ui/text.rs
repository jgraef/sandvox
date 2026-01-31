use std::ops::Range;

use bevy_ecs::{
    component::Component,
    entity::Entity,
    query::{
        Changed,
        Or,
        QueryData,
        Without,
    },
    schedule::IntoScheduleConfigs,
    system::{
        Commands,
        Local,
        Populated,
        Res,
    },
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
        DefaultFont,
        text::{
            Text,
            TextColor,
            TextSize,
        },
    },
    ui::{
        FinalLayout,
        LayoutCache,
        LeafMeasure,
        Root,
        UiSystems,
        render::RenderBufferBuilder,
        view::View,
    },
};

pub(super) fn setup_text_systems(builder: &mut WorldBuilder) {
    builder.add_systems(
        schedule::Render,
        (
            compute_text_layouts.in_set(UiSystems::Layout),
            request_redraw.before(UiSystems::Render),
            render_texts.in_set(UiSystems::Render),
        ),
    );
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TextLeafMeasure;

impl LeafMeasure for TextLeafMeasure {
    type Data = Res<'static, DefaultFont>;
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

fn request_redraw(
    nodes: Populated<&Root, Or<(Changed<TextBuffer>, Changed<TextSize>)>>,
    mut views: Populated<&mut View>,
) {
    for root in nodes {
        let mut view = views.get_mut(root.root).unwrap();
        view.render = true;
    }
}

fn render_texts(
    font: Res<DefaultFont>,
    nodes: Populated<(
        Entity,
        &Text,
        &TextBuffer,
        Option<&TextSize>,
        Option<&TextColor>,
        &FinalLayout,
        &Root,
    )>,
    mut views: Populated<(&View, &mut RenderBufferBuilder)>,
) {
    let displacement = font.glyph_displacement();

    for (entity, text, text_buffer, text_size, text_color, final_layout, root) in nodes {
        let (view, mut render_buffer_builder) = views.get_mut(root.root).unwrap();

        if view.render {
            let content_offset =
                Vector2::new(final_layout.content_box_x(), final_layout.content_box_y());
            let content_size = Vector2::new(
                final_layout.content_box_width(),
                final_layout.content_box_height(),
            );

            let text_size = text_size.copied().unwrap_or_default().scaling;
            let displacement = displacement * text_size;
            let width_constraint = (content_size.x / displacement.x).floor().max(0.0) as usize;

            let text_color = text_color.copied().map(|color| color.color);

            tracing::trace!(?entity, text = ?text.text, ?content_offset, ?content_size, depth = ?final_layout.depth, "render text");

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
                                // we have these available in the shader, so we could add this there
                                // (we used to do this).
                                let (glyph_offset, glyph_size) = font.glyph_bbox(glyph_id);

                                render_buffer_builder
                                    .push_quad(
                                        glyph_offset.cast::<f32>() * text_size + offset,
                                        glyph_size.cast::<f32>() * text_size,
                                        final_layout.depth,
                                        text_color,
                                    )
                                    .set_glyph_texture(glyph_id);

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
        }
    }
}

/// System that calculates text layouts
///
/// The layout doesn't contain positions for the glyphs yet. This is done when
/// the text measure function runs.
fn compute_text_layouts(
    font: Res<DefaultFont>,
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
