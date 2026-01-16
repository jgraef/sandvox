use bevy_ecs::{
    change_detection::{
        DetectChanges,
        Mut,
    },
    component::Component,
    entity::Entity,
    query::{
        Changed,
        Or,
        Without,
    },
    resource::Resource,
    system::{
        Commands,
        Populated,
        Query,
        Res,
        ResMut,
    },
};
use palette::{
    Srgba,
    WithAlpha,
};

use crate::ui::{
    LeafMeasure,
    RoundedLayout,
};

pub(super) fn text_measure_system(
    mut fonts: ResMut<Fonts>,
    mut text_nodes: Query<(Mut<Buffer>, &mut LeafMeasure), Changed<LeafMeasure>>,
) {
    // https://github.com/DioxusLabs/taffy/blob/f8a32fcfd47956ccee10ddc28273edab82e002ad/examples/cosmic_text/src/main.rs#L20

    for (mut buffer, mut leaf_measure) in text_nodes.iter_mut() {
        leaf_measure.respond_with(|known_dimensions, available_space| {
            let text_area_size_changed =
                (known_dimensions.width, known_dimensions.height) != buffer.buffer.size();
            let buffer_changed = buffer.is_changed();

            tracing::debug!(?text_area_size_changed, ?buffer_changed);

            if text_area_size_changed {
                let width_constraint = known_dimensions.width.or(match available_space.width {
                    taffy::AvailableSpace::MinContent => Some(0.0),
                    taffy::AvailableSpace::MaxContent => None,
                    taffy::AvailableSpace::Definite(width) => Some(width),
                });
                tracing::debug!(?width_constraint);
                buffer
                    .buffer
                    .set_size(&mut fonts.font_system, width_constraint, None);
            }

            if buffer_changed || text_area_size_changed {
                buffer
                    .buffer
                    .shape_until_scroll(&mut fonts.font_system, false);

                let mut width = 0.0f32;
                let mut total_lines = 0;
                for layout_run in buffer.buffer.layout_runs() {
                    width = width.max(layout_run.line_w);
                    total_lines += 1;
                }

                let height = total_lines as f32 * buffer.buffer.metrics().line_height;

                tracing::debug!(?width, ?height);

                Some(taffy::Size { width, height })
            }
            else {
                None
            }
        });
    }
}

pub(super) fn render_text_nodes(
    fonts: Res<Fonts>,
    nodes: Query<(&RoundedLayout, &Buffer, &FontColor)>,
) {
    for (rounded_layout, buffer, font_color) in nodes.iter() {
        if !rounded_layout.is_visible() {
            continue;
        }

        //tracing::debug!(?rounded_layout, "text node");

        //let mut ui_render_pass = ui_render_passes.get_mut(document_root.0).unwrap();
        //let origin = point_from_taffy(rounded_layout.0.location);

        for layout_run in buffer.buffer.layout_runs() {
            for glyph in layout_run.glyphs {
                /*let physical_glyph =
                    glyph.physical((rounded_layout.location.x, rounded_layout.location.y), 1.0);

                let position = Point2::new(physical_glyph.x, physical_glyph.y);*/

                /*let hitbox = AaQuad::from_top_left_and_size(
                    &(origin + Vector2::new(glyph.x, glyph.y)),
                    &Vector2::new(glyph.w, buffer.buffer.metrics().line_height),
                );
                ui_render_pass.draw_quad(
                    &hitbox.quad(),
                    Some(&font_color.color),
                    &white_texture.texture,
                    rounded_layout.0.order,
                );*/
            }
        }
    }
}
