use std::{
    collections::HashMap,
    num::NonZero,
    path::Path,
};

use bytemuck::{
    Pod,
    Zeroable,
};
use image::RgbaImage;
use nalgebra::{
    Point2,
    Vector2,
};

use crate::{
    render::staging::Staging,
    wgpu::{
        blit::{
            Blitter,
            BlitterTransaction,
        },
        buffer::TypedArrayBuffer,
        image::{
            ImageTextureExt,
            MipLevels,
            UnsupportedColorSpace,
        },
    },
};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Atlas full ({limit}x{limit})")]
    AtlasFull { limit: u32 },

    #[error("Image too large: {}x{} > {limit}x{limit}", .image_size.x, .image_size.y)]
    TooLarge {
        image_size: Vector2<u32>,
        limit: u32,
    },

    #[error(transparent)]
    Image(#[from] image::ImageError),

    #[error(transparent)]
    UnsupportedColorSpace(#[from] UnsupportedColorSpace),
}

#[derive(Clone, Copy, Debug)]
pub struct AtlasConfig {
    pub initial_size: u32,
    pub initial_data_buffer_size: NonZero<usize>,
    pub size_limit: Option<u32>,
    pub format: wgpu::TextureFormat,
    pub usage: wgpu::TextureUsages,
}

impl Default for AtlasConfig {
    fn default() -> Self {
        Self {
            initial_size: 1024,
            initial_data_buffer_size: const { NonZero::new(256).unwrap() },
            size_limit: None,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING,
        }
    }
}

#[derive(derive_more::Debug)]
pub struct Atlas {
    #[debug(skip)]
    allocator: guillotiere::AtlasAllocator,
    size: u32,
    size_limit: u32,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
    entries: Vec<Option<Entry>>,
    num_entries: usize,
    free_list: Vec<AtlasId>,
    changes: Vec<Change>,
    blitter: Blitter,
    samplers: HashMap<SamplerMode, wgpu::Sampler>,
    version: AtlasVersion,
    atlas_texture: wgpu::TextureView,
    data_buffer: TypedArrayBuffer<DataBufferItem>,
}

impl Atlas {
    pub fn new(device: &wgpu::Device, config: AtlasConfig) -> Self {
        let AtlasConfig {
            initial_size,
            initial_data_buffer_size,
            size_limit,
            format,
            mut usage,
        } = config;

        let size_limit = size_limit.unwrap_or_else(|| {
            // lets hope this is optimized, and won't copy the whole struct... well :shrug:
            device.limits().max_texture_dimension_2d
        });

        assert!(initial_size > 0 && initial_size.is_power_of_two());
        assert!(size_limit >= initial_size);

        let allocator =
            guillotiere::AtlasAllocator::new(vector2_to_guillotiere(Vector2::repeat(initial_size)));

        let blitter = Blitter::new(device);

        // required for blitting to it
        usage |= wgpu::TextureUsages::RENDER_ATTACHMENT;

        // for debugging
        usage |= wgpu::TextureUsages::COPY_SRC;

        let atlas_texture = allocate_atlas_texture(device, initial_size, format, usage);

        let data_buffer = TypedArrayBuffer::with_capacity(
            device.clone(),
            "atlas data",
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            initial_data_buffer_size.get(),
        );

        Self {
            allocator,
            size: initial_size,
            size_limit,
            format,
            usage,
            entries: vec![],
            num_entries: 0,
            free_list: vec![],
            changes: vec![],
            blitter,
            samplers: HashMap::default(),
            version: Default::default(),
            atlas_texture,
            data_buffer,
        }
    }

    fn insert_entry(&mut self, entry: Entry) -> AtlasId {
        self.num_entries += 1;

        if let Some(atlas_id) = self.free_list.pop() {
            let slot = &mut self.entries[atlas_id.to_index()];
            assert!(slot.is_none());
            *slot = Some(entry);
            atlas_id
        }
        else {
            let index = self.entries.len();
            self.entries.push(Some(entry));
            AtlasId(index.try_into().unwrap())
        }
    }

    fn allocate(
        &mut self,
        size: Vector2<u32>,
        padding: Padding,
        change_index: Option<usize>,
    ) -> Result<AtlasId, Error> {
        // check if image won't ever fit
        if size.x > self.size_limit || size.y > self.size_limit {
            return Err(Error::TooLarge {
                image_size: size,
                limit: self.size_limit,
            });
        }

        let padded_size = size + padding.additional_size();

        // allocate space for image
        let (outer_offset, alloc_id) = loop {
            if let Some(allocation) = self.allocator.allocate(vector2_to_guillotiere(padded_size)) {
                let min = guillotiere_to_point2(allocation.rectangle.min);
                let max = guillotiere_to_point2(allocation.rectangle.max);

                let outer_offset = min;
                let outer_size = max - min;
                assert_eq!(outer_size, padded_size);

                break (outer_offset, allocation.id);
            }
            else if self.size < self.size_limit {
                // todo: make sure the new size fits the requested size
                let new_size = (2 * self.size).min(self.size_limit);
                self.allocator
                    .grow(vector2_to_guillotiere(Vector2::repeat(new_size)));
                self.size = new_size;
            }
            else {
                return Err(Error::AtlasFull { limit: self.size });
            }
        };

        let atlas_id = self.insert_entry(Entry {
            alloc_id,
            pending_change: change_index,
            outer_offset,
            outer_size: padded_size,
            inner_offset: outer_offset + padding.inner_offset(),
            inner_size: size,
        });

        Ok(atlas_id)
    }

    pub fn insert_texture(
        &mut self,
        texture_view: wgpu::TextureView,
        padding: Padding,
        sampler_mode: SamplerMode,
    ) -> Result<AtlasId, Error> {
        let texture_size = texture_view.texture().size();
        let texture_size = Vector2::new(texture_size.width, texture_size.height);

        let change_index = self.changes.len();
        let id = self.allocate(texture_size, padding, Some(change_index))?;

        self.changes.push(Change::Insert {
            id,
            source_texture: texture_view,
            source_offset: Point2::origin(),
            source_size: texture_size,
            padding,
            sampler_mode,
        });

        Ok(id)
    }

    pub fn insert_image(
        &mut self,
        image: &RgbaImage,
        padding: Padding,
        sampler_mode: SamplerMode,
        device: &wgpu::Device,
        staging: &mut Staging,
    ) -> Result<AtlasId, Error> {
        // upload image to gpu
        let texture = image.create_texture(
            "atlas insert",
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            MipLevels::One,
            device,
            staging,
        )?;

        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("atlas insert"),
            ..Default::default()
        });

        self.insert_texture(texture_view, padding, sampler_mode)
    }

    pub fn remove(&mut self, id: AtlasId) {
        let entry = self.entries[id.to_index()].take().unwrap();
        self.num_entries -= 1;

        self.free_list.push(id);
        self.allocator.deallocate(entry.alloc_id);
    }

    pub fn flush(&mut self, device: &wgpu::Device, mut staging: &mut Staging) -> bool {
        let mut new_texture = false;
        let new_data_buffer;

        // note: we might potentially want to change this check when we implement atlas
        // rearranging or some other changes
        if self.changes.is_empty() {
            // if there aren't any changes, early exit
            return false;
        }

        // blit any changes
        {
            let old_atlas_size = self.atlas_texture.texture().width();

            if self.size != old_atlas_size {
                assert!(self.size > old_atlas_size);

                let atlas_texture =
                    allocate_atlas_texture(device, self.size, self.format, self.usage);

                let mut blitter = AtlasBlitterTransaction {
                    inner: self.blitter.begin(&atlas_texture),
                    samplers: &mut self.samplers,
                    device,
                };

                blitter.blit_all_entries(&mut self.changes, &mut self.entries, &self.atlas_texture);
                blitter.finish(device, &mut staging);

                self.atlas_texture = atlas_texture;
                new_texture = true;
            }
            else {
                let mut blitter = AtlasBlitterTransaction {
                    inner: self.blitter.begin(&self.atlas_texture),
                    samplers: &mut self.samplers,
                    device,
                };

                blitter.blit_changes(&self.changes, &mut self.entries);
                blitter.finish(device, &mut staging);
            }
        }

        // update data buffer
        {
            let atlas_size_inv = 1.0 / (self.size as f32);

            new_data_buffer = self.data_buffer.write_all_with(
                self.entries.len(),
                |view: &mut [DataBufferItem]| {
                    for (target, source) in view
                        .iter_mut()
                        .zip(self.entries.iter().filter_map(|entry| entry.as_ref()))
                    {
                        *target = DataBufferItem {
                            uv_offset: atlas_size_inv * source.inner_offset.cast::<f32>(),
                            uv_size: atlas_size_inv * source.inner_size.cast::<f32>(),
                        };
                    }
                },
                |_new_buffer| {},
                &mut staging,
            );
        }

        // dump atlas texture for debugging
        {
            let path = Path::new("tmp/atlas.png");
            let size = self.size;
            tracing::debug!(path = %path.display(), ?size, "dumping texture atlas");

            let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("atlas read-back staging"),
                size: (size * size * 4) as wgpu::BufferAddress,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });

            staging.command_encoder_mut().copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: self.atlas_texture.texture(),
                    mip_level: 0,
                    origin: Default::default(),
                    aspect: Default::default(),
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &staging_buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(size * 4),
                        rows_per_image: None,
                    },
                },
                wgpu::Extent3d {
                    width: size,
                    height: size,
                    depth_or_array_layers: 1,
                },
            );

            staging
                .command_encoder_mut()
                .on_submitted_work_done(move || {
                    staging_buffer
                        .clone()
                        .map_async(wgpu::MapMode::Read, .., move |result| {
                            result.unwrap();
                            let mapped_range = staging_buffer.get_mapped_range(..);

                            let image = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(
                                size, size,
                                &*mapped_range,
                            )
                            .unwrap();

                            if let Err(error) = image.save(path) {
                                tracing::error!(path = %path.display(), "couldn't save texture atlas: {error}");
                            }
                        });
                });
        }

        self.changes.clear();

        if new_texture || new_data_buffer {
            self.version.0 += 1;
            true
        }
        else {
            false
        }
    }

    pub fn version(&self) -> AtlasVersion {
        self.version
    }

    pub fn resources(&self) -> AtlasResources<'_> {
        AtlasResources {
            texture: &self.atlas_texture,
            data_buffer: self.data_buffer.buffer(),
            version: self.version,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, derive_more::Into, Pod, Zeroable)]
#[repr(C)]
pub struct AtlasId(u32);

impl AtlasId {
    fn to_index(&self) -> usize {
        self.0.try_into().unwrap()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct AtlasResources<'a> {
    pub texture: &'a wgpu::TextureView,
    pub data_buffer: &'a wgpu::Buffer,
    pub version: AtlasVersion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct AtlasVersion(usize);

#[derive(Debug)]
struct Entry {
    alloc_id: guillotiere::AllocId,
    pending_change: Option<usize>,
    outer_offset: Point2<u32>,
    outer_size: Vector2<u32>,
    inner_offset: Point2<u32>,
    inner_size: Vector2<u32>,
}

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
struct DataBufferItem {
    uv_offset: Point2<f32>,
    uv_size: Vector2<f32>,
}

#[derive(Debug)]
enum Change {
    Insert {
        id: AtlasId,
        source_texture: wgpu::TextureView,
        source_offset: Point2<u32>,
        source_size: Vector2<u32>,
        padding: Padding,
        sampler_mode: SamplerMode,
    },
}

#[derive(Clone, Copy, Default, Debug)]
pub struct Padding {
    pub left: u32,
    pub right: u32,
    pub top: u32,
    pub bottom: u32,
}

impl Padding {
    pub fn uniform(padding: u32) -> Self {
        Self {
            left: padding,
            right: padding,
            top: padding,
            bottom: padding,
        }
    }

    pub fn additional_size(&self) -> Vector2<u32> {
        Vector2::new(self.left + self.right, self.top + self.bottom)
    }

    pub fn inner_offset(&self) -> Vector2<u32> {
        Vector2::new(self.left, self.top)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SamplerMode {
    pub address_mode_u: wgpu::AddressMode,
    pub address_mode_v: wgpu::AddressMode,
}

impl SamplerMode {
    pub const RESIZE: Self = Self::both(wgpu::AddressMode::ClampToEdge);

    pub const fn both(address_mode: wgpu::AddressMode) -> Self {
        Self {
            address_mode_u: address_mode,
            address_mode_v: address_mode,
        }
    }
}

fn vector2_to_guillotiere(size: Vector2<u32>) -> guillotiere::Size {
    guillotiere::Size::new(
        i32::try_from(size.x).unwrap(),
        i32::try_from(size.y).unwrap(),
    )
}

fn guillotiere_to_point2(point: guillotiere::Point) -> Point2<u32> {
    Point2::new(point.x.try_into().unwrap(), point.y.try_into().unwrap())
}

fn get_sampler<'a>(
    samplers: &'a mut HashMap<SamplerMode, wgpu::Sampler>,
    device: &wgpu::Device,
    sampler_mode: SamplerMode,
) -> &'a wgpu::Sampler {
    samplers.entry(sampler_mode).or_insert_with(|| {
        device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some(&format!("atlas sampler {sampler_mode:?}")),
            address_mode_u: sampler_mode.address_mode_u,
            address_mode_v: sampler_mode.address_mode_v,
            ..Default::default()
        })
    })
}

fn allocate_atlas_texture(
    device: &wgpu::Device,
    size: u32,
    format: wgpu::TextureFormat,
    usage: wgpu::TextureUsages,
) -> wgpu::TextureView {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("atlas"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    });

    texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("atlas"),
        ..Default::default()
    })
}

#[derive(Debug)]
struct AtlasBlitterTransaction<'a> {
    inner: BlitterTransaction<'a>,
    samplers: &'a mut HashMap<SamplerMode, wgpu::Sampler>,
    device: &'a wgpu::Device,
}

impl<'a> AtlasBlitterTransaction<'a> {
    fn keep(&mut self, old_atlas_texture: &wgpu::TextureView, entry: &Entry) {
        let sampler = get_sampler(self.samplers, self.device, SamplerMode::RESIZE);

        self.inner.blit(
            old_atlas_texture,
            sampler,
            entry.outer_offset.cast::<i32>(),
            entry.outer_size,
            entry.outer_offset.cast::<i32>(),
            entry.outer_size,
        );
    }

    fn insert(
        &mut self,
        source_texture: &wgpu::TextureView,
        source_offset: Point2<u32>,
        source_size: Vector2<u32>,
        padding: Padding,
        sampler_mode: SamplerMode,
        outer_offset: Point2<u32>,
        outer_size: Vector2<u32>,
    ) {
        let source_sampler = get_sampler(self.samplers, self.device, sampler_mode);

        self.inner.blit(
            source_texture,
            source_sampler,
            source_offset.cast::<i32>() - padding.inner_offset().cast::<i32>(),
            source_size + padding.additional_size(),
            outer_offset.cast(),
            outer_size,
        );
    }

    fn blit_change<'e>(
        &mut self,
        change: &Change,
        mut get_entry_rect: impl FnMut(AtlasId) -> (Point2<u32>, Vector2<u32>),
    ) {
        match change {
            Change::Insert {
                id,
                source_texture,
                source_offset,
                source_size,
                padding,
                sampler_mode,
            } => {
                let (entry_offset, entry_size) = get_entry_rect(*id);
                self.insert(
                    source_texture,
                    *source_offset,
                    *source_size,
                    *padding,
                    *sampler_mode,
                    entry_offset,
                    entry_size,
                );
            }
        }
    }

    fn blit_changes(&mut self, changes: &[Change], entries: &mut [Option<Entry>]) {
        for change in changes {
            self.blit_change(change, |id| {
                let entry = entries[id.to_index()].as_mut().unwrap();
                entry.pending_change = None;
                (entry.outer_offset, entry.outer_size)
            })
        }
    }

    fn blit_all_entries(
        &mut self,
        changes: &mut [Change],
        entries: &mut [Option<Entry>],
        old_altas_texture: &wgpu::TextureView,
    ) {
        for entry in entries {
            if let Some(entry) = entry {
                if let Some(change_index) = entry.pending_change.take() {
                    let change = &changes[change_index];
                    self.blit_change(change, |_id| (entry.outer_offset, entry.outer_size));
                }
                else {
                    self.keep(old_altas_texture, entry);
                }
            }
        }
    }

    pub fn finish(self, device: &wgpu::Device, staging: &mut Staging) {
        self.inner.finish(device, staging);
    }
}
