use bevy_ecs::{
    change_detection::{
        DetectChanges,
        Ref,
    },
    component::Component,
    entity::Entity,
    name::NameOrEntity,
    query::Without,
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
                text_layout.in_set(UiSystems::Layout),
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
                    /*let width_constraint = known_dimensions.width.or(match available_space.width {
                        AvailableSpace::MinContent => Some(0.0),
                        AvailableSpace::MaxContent => None,
                        AvailableSpace::Definite(width) => Some(width),
                    });*/
                    // todo
                    let _ = (known_dimensions, available_space);
                    let width_constraint = None;

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
