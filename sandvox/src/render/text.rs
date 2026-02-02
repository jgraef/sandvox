use std::{
    collections::HashMap,
    path::Path,
};

use bevy_ecs::component::Component;
use bytemuck::{
    Pod,
    Zeroable,
};
use color_eyre::eyre::Error;
use nalgebra::{
    Point2,
    Vector2,
};
use palette::{
    Srgba,
    WithAlpha,
};

use crate::{
    render::{
        staging::Staging,
        text::bdf::make_font_sheet,
    },
    wgpu::{
        TextureSourceLayout,
        buffer::WriteStaging,
    },
};

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
    pub scaling: f32,
}

impl Default for TextSize {
    fn default() -> Self {
        Self { scaling: 1.0 }
    }
}

#[derive(Debug)]
pub struct Font {
    data: FontData,
    texture: wgpu::TextureView,
    data_buffer: wgpu::Buffer,
}

impl Font {
    pub fn open(
        path: impl AsRef<Path>,
        device: &wgpu::Device,
        staging: &mut Staging,
    ) -> Result<Self, Error> {
        let bdf_data = std::fs::read_to_string(&path)?;
        let (data, image) = make_font_sheet(&bdf_data)?;

        // create data buffer containing offsets and uvs for glyphs
        let data_buffer = {
            let data_buffer_size = (size_of::<FontDataBufferHeader>()
                + size_of::<Glyph>() * data.glyphs.len())
                as wgpu::BufferAddress;

            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
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
                    num_glyphs: data.glyphs.len().try_into().unwrap(),
                    _padding: 0,
                    atlas_size: data.atlas_size,
                };

                let view_glyphs: &mut [Glyph] =
                    bytemuck::cast_slice_mut(&mut view[size_of::<FontDataBufferHeader>()..]);
                view_glyphs.copy_from_slice(&*data.glyphs);
            }

            buffer.unmap();

            buffer
        };

        // create texture of glyph atlas
        let texture = {
            let size = wgpu::Extent3d {
                width: image.width(),
                height: image.height(),
                depth_or_array_layers: 1,
            };

            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("font"),
                size,
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });

            let mut view = staging.write_texture(
                TextureSourceLayout {
                    bytes_per_row: image.width(),
                    rows_per_image: None,
                },
                wgpu::TexelCopyTextureInfo {
                    texture: &texture,
                    mip_level: 0,
                    origin: Default::default(),
                    aspect: Default::default(),
                },
                size,
            );

            view.copy_from_slice(&image);

            texture.create_view(&wgpu::TextureViewDescriptor {
                label: Some("font"),
                ..Default::default()
            })
        };

        Ok(Self {
            data,
            texture,
            data_buffer,
        })
    }

    pub fn glyph_id(&self, character: char) -> Option<GlyphId> {
        self.data.codepoints.get(&character).copied()
    }

    pub fn glyph_id_or_replacement(&self, character: char) -> Option<GlyphId> {
        self.glyph_id(character).or(self.data.replacement_glyph)
    }

    pub fn replacement_glyph(&self) -> Option<GlyphId> {
        self.data.replacement_glyph
    }

    pub fn contains(&self, character: char) -> bool {
        self.data.codepoints.contains_key(&character)
    }

    pub fn glyph_displacement(&self) -> Vector2<f32> {
        self.data.glyph_displacement
    }

    pub fn glyph_bbox(&self, glyph_id: GlyphId) -> (Point2<u32>, Vector2<u32>) {
        let glyph = &self.data.glyphs[glyph_id.to_index()];
        (glyph.offset.into(), glyph.size)
    }

    pub fn resources(&self) -> FontResources<'_> {
        FontResources {
            texture: &self.texture,
            data_buffer: &self.data_buffer,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct FontResources<'a> {
    pub texture: &'a wgpu::TextureView,
    pub data_buffer: &'a wgpu::Buffer,
}

#[derive(Clone, Debug)]
struct FontData {
    glyphs: Vec<Glyph>,
    codepoints: HashMap<char, GlyphId>,
    replacement_glyph: Option<GlyphId>,
    glyph_displacement: Vector2<f32>,
    atlas_size: Vector2<u32>,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, derive_more::Into)]
pub struct GlyphId(u32);

impl GlyphId {
    fn to_index(&self) -> usize {
        self.0 as usize
    }
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
        GlyphId,
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

    #[inline]
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

            // make sure width is a multiple of 256 as GPUs tend to use a multiple of it as
            // row-stride (I think)
            sheet_size.x = sheet_size
                .x
                .next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);

            glyphs_per_row = (sheet_size.x - padding.x) / padded_cell_size.x;
            let num_rows = (num_glyphs as u32).div_ceil(glyphs_per_row);
            sheet_size.y = num_rows * padded_cell_size.y + padding.y;

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

    pub(super) fn make_font_sheet(bdf_data: &str) -> Result<(FontData, GrayImage), Error> {
        const LUMA_FG: Luma<u8> = Luma([255]);
        const LUMA_BG: Luma<u8> = Luma([0]);

        let font = bdf_parser::Font::parse(&bdf_data)?;

        // count glyphs with codepoints
        let num_glyphs = font
            .glyphs
            .iter()
            .filter(|glyph| matches!(glyph.encoding, bdf_parser::Encoding::Standard(_)))
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
                font.metadata
                    .properties
                    .try_get::<i32>(bdf_parser::Property::FigureWidth)
                    .unwrap()
                    .unwrap() as f32,
                font.metadata
                    .properties
                    .try_get::<i32>(bdf_parser::Property::PixelSize)
                    .unwrap()
                    .unwrap() as f32,
            ),
            replacement_glyph: font
                .metadata
                .properties
                .try_get::<i32>(bdf_parser::Property::DefaultChar)
                .ok()
                .flatten()
                .map(|glyph_id| GlyphId(glyph_id as u32)),
        };
        let mut i = 0;

        for glyph in font.glyphs.iter() {
            if let bdf_parser::Encoding::Standard(encoding) = glyph.encoding
                && let Some(character) = char::from_u32(encoding)
            {
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

                font_data.codepoints.insert(character, GlyphId(i));

                for y in 0..glyph_size.y {
                    for x in 0..glyph_size.x {
                        let pixel = glyph.pixel(x as usize, y as usize).unwrap_or_default();
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
