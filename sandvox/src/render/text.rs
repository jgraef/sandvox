use std::{
    collections::HashMap,
    path::PathBuf,
};

use bevy_ecs::{
    component::Component,
    resource::Resource,
    schedule::{
        IntoScheduleConfigs,
        SystemSet,
    },
    system::{
        Commands,
        InMut,
        IntoSystem,
        Res,
    },
};
use bytemuck::{
    Pod,
    Zeroable,
};
use color_eyre::eyre::Error;
use image::GrayImage;
use nalgebra::Vector2;
use palette::{
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
        text::bdf::make_font_sheet,
    },
    wgpu::WgpuContext,
};

#[derive(Clone, Debug, Default)]
pub struct FontPlugin {
    pub font: PathBuf,
}

impl Plugin for FontPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        let bdf_data = std::fs::read(&self.font)?;
        let (font_data, font_image) = make_font_sheet(&bdf_data)?;

        // for debugging
        let _ = font_image.save("tmp/font.png");

        builder.insert_resource(font_data).add_systems(
            schedule::Startup,
            (
                setup_bind_group_layout.in_set(FontSystems::Setup),
                create_font_texture
                    .with_input(font_image)
                    .in_set(FontSystems::LoadFonts)
                    .after(FontSystems::Setup),
            )
                .in_set(RenderSystems::Setup),
        );

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, SystemSet)]
pub enum FontSystems {
    Setup,
    LoadFonts,
}

fn setup_bind_group_layout(wgpu: Res<WgpuContext>, mut commands: Commands) {
    let bind_group_layout =
        wgpu.device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("text rendering"),
                entries: &[
                    // font glyph data
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // texture
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
                    // sampler
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                ],
            });

    commands.insert_resource(FontBindGroupLayout { bind_group_layout });
}

#[derive(Debug, Resource)]
pub struct FontBindGroupLayout {
    pub bind_group_layout: wgpu::BindGroupLayout,
}

/// system that creates a bind group for the font
///
/// todo: make fonts entities/components so we can have multiple
fn create_font_texture(
    InMut(font_image): InMut<GrayImage>,
    font_data: Res<Font>,
    wgpu: Res<WgpuContext>,
    font_bind_group_layout: Res<FontBindGroupLayout>,
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
        layout: &font_bind_group_layout.bind_group_layout,
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

#[derive(Debug, Resource)]
pub struct FontBindGroup {
    pub bind_group: wgpu::BindGroup,
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

#[derive(Clone, Debug, Resource)]
pub struct Font {
    glyphs: Vec<Glyph>,
    codepoints: HashMap<char, u32>,
    replacement_glyph: Option<u32>,

    glyph_displacement: Vector2<f32>,

    atlas_size: Vector2<u32>,
}

impl Font {
    pub fn glyph_id(&self, character: char) -> Option<u32> {
        self.codepoints
            .get(&character)
            .copied()
            .or(self.replacement_glyph)
    }

    pub fn contains(&self, character: char) -> bool {
        self.codepoints.contains_key(&character)
    }

    pub fn glyph_displacement(&self) -> Vector2<f32> {
        self.glyph_displacement
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
        Font,
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

    pub(super) fn make_font_sheet(bdf_data: &[u8]) -> Result<(Font, GrayImage), Error> {
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
        let mut font_data = Font {
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
