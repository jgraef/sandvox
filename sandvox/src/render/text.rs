use std::{
    collections::HashMap,
    ops::Range,
    path::PathBuf,
};

use bevy_ecs::{
    component::Component,
    entity::Entity,
    name::NameOrEntity,
    query::{
        Changed,
        Or,
        Without,
    },
    resource::Resource,
    schedule::IntoScheduleConfigs,
    system::{
        Commands,
        InMut,
        IntoSystem,
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
use image::GrayImage;
use nalgebra::{
    Vector2,
    Vector4,
};
use palette::{
    LinSrgba,
    Srgba,
    WithAlpha,
};
use wgpu::util::DeviceExt;

use crate::{
    ecs::{
        plugin::{
            Plugin,
            WorldBuilder,
        },
        schedule,
    },
    render::{
        RenderSystems,
        frame::{
            Frame,
            FrameUniformLayout,
        },
        staging::Staging,
        surface::Surface,
        text::bdf::make_font_sheet,
    },
    wgpu::{
        WgpuContext,
        buffer::TypedArrayBuffer,
    },
};

#[derive(Clone, Debug, Default)]
pub struct TextPlugin {
    pub font: PathBuf,
}

impl Plugin for TextPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        let bdf_data = std::fs::read(&self.font)?;
        let (font_data, font_image) = make_font_sheet(&bdf_data)?;

        // for debugging
        let _ = font_image.save("tmp/font.png");

        builder
            .insert_resource(font_data)
            .add_systems(
                schedule::Startup,
                (
                    create_text_render_pipeline_shared,
                    create_font_texture.with_input(font_image),
                )
                    .chain()
                    .in_set(RenderSystems::Setup),
            )
            .add_systems(
                schedule::Render,
                (
                    (
                        create_text_render_pipeline_for_surfaces,
                        create_and_update_text_buffers,
                    )
                        .in_set(RenderSystems::BeginFrame),
                    render_text.in_set(RenderSystems::RenderUi),
                ),
            );
        Ok(())
    }
}

#[derive(Debug, Resource)]
struct TextRenderPipelineShared {
    font_bind_group_layout: wgpu::BindGroupLayout,
    pipeline_layout: wgpu::PipelineLayout,
    shader: wgpu::ShaderModule,
}

#[derive(Debug, Component)]
struct TextRenderPipelinePerSurface {
    pipeline: wgpu::RenderPipeline,
}

fn create_text_render_pipeline_shared(
    wgpu: Res<WgpuContext>,
    frame_uniform_layout: Res<FrameUniformLayout>,
    mut commands: Commands,
) {
    let font_bind_group_layout =
        wgpu.device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("text rendering"),
                entries: &[
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

    let pipeline_layout = wgpu
        .device
        .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("text rendering"),
            bind_group_layouts: &[
                &frame_uniform_layout.bind_group_layout,
                &font_bind_group_layout,
            ],
            immediate_size: 0,
        });

    let shader = wgpu
        .device
        .create_shader_module(wgpu::include_wgsl!("text.wgsl"));

    commands.insert_resource(TextRenderPipelineShared {
        font_bind_group_layout,
        pipeline_layout,
        shader,
    });
}

fn create_text_render_pipeline_for_surfaces(
    wgpu: Res<WgpuContext>,
    shared: Res<TextRenderPipelineShared>,
    surfaces: Populated<(Entity, NameOrEntity, &Surface), Without<TextRenderPipelinePerSurface>>,
    mut commands: Commands,
) {
    for (entity, name, surface) in surfaces {
        tracing::trace!(surface = %name, "creating mesh render pipeline for surface");

        let pipeline = wgpu
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("text rendering"),
                layout: Some(&shared.pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shared.shader,
                    entry_point: Some("text_vertex"),
                    compilation_options: Default::default(),
                    buffers: &[GlyphVertex::LAYOUT],
                },
                primitive: wgpu::PrimitiveState {
                    topology: wgpu::PrimitiveTopology::TriangleList,
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None,
                    unclipped_depth: false,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    //polygon_mode: wgpu::PolygonMode::Line,
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
                    module: &shared.shader,
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
            .entity(entity)
            .insert(TextRenderPipelinePerSurface { pipeline });
    }
}

/// system that creates a bind group for the font
///
/// todo: make fonts entities/components so we can have multiple
fn create_font_texture(
    InMut(font_image): InMut<GrayImage>,
    font_data: Res<FontData>,
    wgpu: Res<WgpuContext>,
    shared: Res<TextRenderPipelineShared>,
    mut commands: Commands,
) {
    // create data buffer containing offsets and uvs for glyphs
    let data_buffer = {
        let data_buffer_size = (size_of::<FontDataBufferHeader>()
            + size_of::<Glyph>() * font_data.glyphs.len())
            as wgpu::BufferAddress;

        let buffer = wgpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("font"),
            size: data_buffer_size,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });

        {
            // fill buffer
            let mut view = buffer.get_mapped_range_mut(..);

            let view_header: &mut FontDataBufferHeader =
                bytemuck::from_bytes_mut(&mut view[..size_of::<FontDataBufferHeader>()]);
            *view_header = FontDataBufferHeader {
                num_glyphs: font_data.glyphs.len().try_into().unwrap(),
                _padding: 0,
                atlas_size: font_data.atlas_size,
            };

            let view_glyphs: &mut [Glyph] =
                bytemuck::cast_slice_mut(&mut view[size_of::<FontDataBufferHeader>()..]);
            view_glyphs.copy_from_slice(&*font_data.glyphs);
        }

        buffer.unmap();

        buffer
    };

    // create texture of glyph atlas
    // todo: use staging
    let texture = wgpu.device.create_texture_with_data(
        &wgpu.queue,
        &wgpu::TextureDescriptor {
            label: Some("font"),
            size: wgpu::Extent3d {
                width: font_image.width(),
                height: font_image.height(),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        },
        wgpu::util::TextureDataOrder::LayerMajor,
        &font_image,
    );

    let texture_view = texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("font"),
        ..Default::default()
    });

    let sampler = wgpu.device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("font"),
        mag_filter: wgpu::FilterMode::Nearest,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    });

    let bind_group = wgpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("font"),
        layout: &shared.font_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: data_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&texture_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&sampler),
            },
        ],
    });

    commands.insert_resource(FontBindGroup { bind_group });
}

/// System that creates text buffers (on GPU) for rendering
fn create_and_update_text_buffers(
    wgpu: Res<WgpuContext>,
    font_data: Res<FontData>,
    texts: Populated<
        (
            Entity,
            &Text,
            Option<&TextColor>,
            Option<&TextSize>,
            Option<&mut TextBuffer>,
        ),
        Or<(Changed<Text>, Changed<TextColor>, Changed<TextSize>)>,
    >,
    mut commands: Commands,
    mut vertex_buffer_data: Local<Vec<GlyphVertex>>,
    mut index_buffer_data: Local<Vec<u32>>,
    mut staging: ResMut<Staging>,
) {
    const QUAD_VERTICES: [Vector2<f32>; 4] = [
        Vector2::new(0.0, 0.0),
        Vector2::new(0.0, 1.0),
        Vector2::new(1.0, 0.0),
        Vector2::new(1.0, 1.0),
    ];
    const QUAD_INDICES: [u32; 6] = [
        0, 1, 2, // 1st tri
        3, 2, 1, // 2nd tri
    ];

    for (entity, text, text_color, text_size, text_buffer) in texts {
        let text_color: LinSrgba<f32> = text_color
            .map(|text_color| text_color.color.into_linear())
            .unwrap_or_default();

        let scaling = text_size.copied().unwrap_or_default().height;

        tracing::trace!(?text.text, ?text_color, ?scaling, "updating text buffer");

        assert!(vertex_buffer_data.is_empty());
        assert!(index_buffer_data.is_empty());

        let mut offset = Vector2::zeros();
        for character in text.text.chars() {
            if let Some((glyph_id, glyph)) = font_data.get_glyph(character) {
                let base_index: u32 = vertex_buffer_data.len().try_into().unwrap();

                let glyph_offset = glyph.offset.cast::<f32>();
                let glyph_size = glyph.size.cast::<f32>();

                vertex_buffer_data.extend(QUAD_VERTICES.map(|quad_vertex| {
                    let position =
                        (quad_vertex.component_mul(&glyph_size) + glyph_offset + offset) * scaling;

                    let uv = quad_vertex;

                    GlyphVertex {
                        position: Vector4::new(position.x, position.y, 0.0, 1.0),
                        color: text_color,
                        //color: test_color,
                        uv,
                        glyph_id,
                    }
                }));

                index_buffer_data.extend(QUAD_INDICES.map(|i| i + base_index));

                offset.x += font_data.glyph_displacement.x;
                //offset.x += glyph.size.x as f32 + 1.0;
            }
        }

        if let Some(mut text_buffer) = text_buffer {
            text_buffer.vertex_buffer.write_all(
                &vertex_buffer_data,
                |_buffer| {
                    // we don't have to do anything if it gets reallocated
                },
                &mut *staging,
            );

            text_buffer.index_buffer.write_all(
                &index_buffer_data,
                |_buffer| {
                    // we don't have to do anything if it gets reallocated
                },
                &mut *staging,
            );

            text_buffer.indices = 0..(index_buffer_data.len() as u32);
        }
        else {
            let vertex_buffer = TypedArrayBuffer::from_slice(
                wgpu.device.clone(),
                format!("text: {entity:?} (vertex buffer)"),
                wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                &vertex_buffer_data,
            );
            let index_buffer = TypedArrayBuffer::from_slice(
                wgpu.device.clone(),
                format!("text: {entity:?} (index buffer)"),
                wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                &index_buffer_data,
            );

            commands.entity(entity).insert(TextBuffer {
                vertex_buffer,
                index_buffer,
                indices: 0..(index_buffer_data.len() as u32),
                base_vertex: 0,
            });
        }

        vertex_buffer_data.clear();
        index_buffer_data.clear();
    }
}

fn render_text(
    font: Res<FontBindGroup>,
    frames: Populated<(&mut Frame, &TextRenderPipelinePerSurface)>,
    texts: Populated<&TextBuffer>,
) {
    for (mut frame, pipeline) in frames {
        // todo: add a SurfaceSize component to the window entity and use that to scale
        // UI (immediate or uniform buffer)

        let render_pass = frame.render_pass_mut();

        render_pass.set_pipeline(&pipeline.pipeline);
        render_pass.set_bind_group(1, Some(&font.bind_group), &[]);

        for text_buffer in &texts {
            render_pass.set_vertex_buffer(0, text_buffer.vertex_buffer.buffer().slice(..));
            render_pass.set_index_buffer(
                text_buffer.index_buffer.buffer().slice(..),
                wgpu::IndexFormat::Uint32,
            );
            render_pass.draw_indexed(text_buffer.indices.clone(), text_buffer.base_vertex, 0..1);
        }
    }
}

#[derive(Debug, Resource)]
struct FontBindGroup {
    bind_group: wgpu::BindGroup,
}

#[derive(Clone, Debug, Default, Component)]
pub struct Text {
    pub text: String,
}

impl From<String> for Text {
    fn from(value: String) -> Self {
        Self { text: value }
    }
}

impl From<&str> for Text {
    fn from(value: &str) -> Self {
        Self {
            text: value.to_owned(),
        }
    }
}

#[derive(Clone, Copy, Debug, Component, derive_more::From, derive_more::Into)]
pub struct TextColor {
    pub color: Srgba<f32>,
}

impl Default for TextColor {
    fn default() -> Self {
        Self {
            color: palette::named::BLACK.into_format().with_alpha(1.0),
        }
    }
}

#[derive(Clone, Copy, Debug, Component)]
pub struct TextSize {
    pub height: f32,
}

impl Default for TextSize {
    fn default() -> Self {
        Self { height: 1.0 }
    }
}

#[derive(Debug, Component)]
struct TextBuffer {
    vertex_buffer: TypedArrayBuffer<GlyphVertex>,
    index_buffer: TypedArrayBuffer<u32>,
    indices: Range<u32>,
    base_vertex: i32,
}

#[derive(Clone, Debug, Resource)]
struct FontData {
    glyphs: Vec<Glyph>,
    codepoints: HashMap<char, u32>,
    replacement_glyph: Option<u32>,

    glyph_displacement: Vector2<f32>,

    atlas_size: Vector2<u32>,
}

impl FontData {
    fn get_glyph(&self, character: char) -> Option<(u32, &Glyph)> {
        self.codepoints
            .get(&character)
            .copied()
            .or(self.replacement_glyph)
            .map(|glyph_id| {
                let glyph = &self.glyphs[glyph_id as usize];
                (glyph_id, glyph)
            })
    }
}

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
struct FontDataBufferHeader {
    num_glyphs: u32,
    _padding: u32,
    atlas_size: Vector2<u32>,
}

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
struct Glyph {
    atlas_offset: Vector2<u32>,
    size: Vector2<u32>,
    offset: Vector2<u32>,
}

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
struct GlyphVertex {
    position: Vector4<f32>,
    color: LinSrgba,
    uv: Vector2<f32>,
    glyph_id: u32,
}

impl GlyphVertex {
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

mod bdf {
    // this might be helpful: https://www.x.org/releases/X11R7.6/doc/xorg-docs/specs/XLFD/xlfd.html#pixel_size

    use std::collections::HashMap;

    use color_eyre::eyre::Error;
    use image::{
        GrayImage,
        Luma,
    };
    use nalgebra::{
        Point2,
        Vector2,
    };

    use crate::render::text::{
        FontData,
        Glyph,
    };

    #[derive(Clone, Copy, Debug, Default)]
    struct Bbox {
        min: Point2<i32>,
        max: Point2<i32>,
    }

    impl Bbox {
        fn size(&self) -> Vector2<u32> {
            assert!(self.max.x >= self.min.x);
            assert!(self.max.y >= self.min.y);
            (self.max - self.min).try_cast().unwrap()
        }
    }

    impl From<bdf_parser::BoundingBox> for Bbox {
        fn from(value: bdf_parser::BoundingBox) -> Self {
            let offset = coord_to_vector2(value.offset).into();
            let size = coord_to_vector2(value.size);
            Self {
                min: offset,
                max: offset + size,
            }
        }
    }

    #[inline(always)]
    fn coord_to_vector2(coord: bdf_parser::Coord) -> Vector2<i32> {
        Vector2::new(coord.x, coord.y)
    }

    #[derive(Debug)]
    struct SheetLayout {
        padding: Vector2<u32>,
        cell_size: Vector2<u32>,
        glyphs_per_row: u32,
        sheet_size: Vector2<u32>,
    }

    impl SheetLayout {
        fn new(num_glyphs: usize, cell_size: Vector2<u32>, padding: Vector2<u32>) -> Self {
            let padded_cell_size = cell_size + padding;

            let mut glyphs_per_row = (num_glyphs as f32).sqrt().floor() as u32;

            let mut sheet_size = Vector2::new(glyphs_per_row * padded_cell_size.x, 0);

            // make sure width is a multiple of 512 as GPUs tend to use a multiple of it as
            // row-stride (I think)
            sheet_size.x = sheet_size.x.next_multiple_of(512);

            glyphs_per_row = sheet_size.x / padded_cell_size.x;
            let num_rows = (num_glyphs as u32).div_ceil(glyphs_per_row);
            sheet_size.y = num_rows * padded_cell_size.y;
            sheet_size += padding;

            Self {
                padding,
                cell_size,
                glyphs_per_row,
                sheet_size,
            }
        }

        fn cell_offset(&self, i: u32) -> Vector2<u32> {
            let cell = Vector2::new(i % self.glyphs_per_row, i / self.glyphs_per_row);
            cell.component_mul(&(self.cell_size + self.padding)) + self.padding
        }
    }

    pub(super) fn make_font_sheet(bdf_data: &[u8]) -> Result<(FontData, GrayImage), Error> {
        const LUMA_FG: Luma<u8> = Luma([255]);
        const LUMA_BG: Luma<u8> = Luma([0]);

        let font = bdf_parser::BdfFont::parse(&bdf_data)?;
        tracing::debug!(metadata = ?font.metadata);

        // count glyphs with codepoints
        let num_glyphs = font
            .glyphs
            .iter()
            .filter(|glyph| glyph.encoding.is_some())
            .count();

        // create sheet layout
        let global_bbox: Bbox = font.metadata.bounding_box.into();
        //assert_eq!(global_bbox.size(), Vector2::new(6, 13));

        let sheet_layout = SheetLayout::new(num_glyphs, global_bbox.size(), Vector2::repeat(1));

        // create sheet image
        let mut font_image = GrayImage::from_pixel(
            sheet_layout.sheet_size.x,
            sheet_layout.sheet_size.y,
            LUMA_BG,
        );

        // create font data
        let mut font_data = FontData {
            glyphs: Vec::with_capacity(num_glyphs as usize),
            codepoints: HashMap::with_capacity(num_glyphs as usize),
            atlas_size: sheet_layout.sheet_size,
            glyph_displacement: Vector2::new(
                font.properties
                    .try_get::<i32>(bdf_parser::Property::FigureWidth)
                    .unwrap() as f32,
                font.properties
                    .try_get::<i32>(bdf_parser::Property::PixelSize)
                    .unwrap() as f32,
            ),
            replacement_glyph: font
                .properties
                .try_get::<i32>(bdf_parser::Property::DefaultChar)
                .ok()
                .map(|glyph_id| glyph_id as u32),
        };
        let mut i = 0;

        for glyph in font.glyphs.iter() {
            if let Some(character) = glyph.encoding {
                let glyph_bbox: Bbox = glyph.bounding_box.into();
                let glyph_size = glyph_bbox.size();

                let glyph_offset = {
                    let glyph_offset = Vector2::new(
                        glyph_bbox.min.x - global_bbox.min.x,
                        global_bbox.max.y - glyph_bbox.max.y,
                    );
                    assert!(glyph_offset.x >= 0);
                    assert!(glyph_offset.y >= 0);

                    glyph_offset.try_cast::<u32>().unwrap()
                };

                let atlas_offset = sheet_layout.cell_offset(i) + glyph_offset;

                font_data.glyphs.push(Glyph {
                    atlas_offset,
                    size: glyph_size,
                    offset: glyph_offset,
                });

                font_data
                    .codepoints
                    .insert(character, i.try_into().unwrap());

                for y in 0..glyph_size.y {
                    for x in 0..glyph_size.x {
                        let pixel = glyph.pixel(x as usize, y as usize);
                        let target_coord = Vector2::new(x, y) + atlas_offset;

                        font_image.put_pixel(
                            target_coord.x,
                            target_coord.y,
                            if pixel { LUMA_FG } else { LUMA_BG },
                        );
                    }
                }

                i += 1;
            }
        }

        Ok((font_data, font_image))
    }
}
