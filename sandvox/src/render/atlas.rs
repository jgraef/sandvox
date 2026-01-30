use std::{
    collections::HashMap,
    fmt::Debug,
    num::NonZero,
    path::Path,
    sync::Arc,
};

use bytemuck::{
    Pod,
    Zeroable,
};
use image::RgbaImage;
use itertools::Itertools;
use nalgebra::{
    Point2,
    Vector2,
};
use palette::LinSrgba;
use parking_lot::Mutex;

use crate::{
    render::staging::Staging,
    util::sparse_vec::SparseVec,
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
            initial_size: 256,
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
    allocations: SparseVec<AllocationId, Allocation>,
    views: SparseVec<ViewId, View>,
    dropped: Arc<Mutex<Dropped>>,
    dropped_buf: Vec<ViewId>,
    changes: Vec<Change>,
    blitter: Blitter,
    samplers: HashMap<SamplerMode, wgpu::Sampler>,
    version: AtlasVersion,
    atlas_texture: wgpu::TextureView,
    data_buffer: TypedArrayBuffer<DataBufferItem>,
}

impl Atlas {
    #[profiling::function]
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
            allocations: Default::default(),
            views: Default::default(),
            dropped: Default::default(),
            dropped_buf: vec![],
            changes: vec![],
            blitter,
            samplers: HashMap::default(),
            version: Default::default(),
            atlas_texture,
            data_buffer,
        }
    }

    fn handle_drops(&mut self) {
        assert!(self.dropped_buf.is_empty());

        {
            // swap dropped list with dropped_buf, so we only lock for a brief amount of
            // time. we do this because freeing allocations might take a bit.

            let mut dropped = self.dropped.lock();
            std::mem::swap(&mut self.dropped_buf, &mut dropped.views);
        }

        for view_id in self.dropped_buf.drain(..) {
            tracing::debug!(?view_id, "removing view");

            let view = self.views.remove(view_id).unwrap();
            let allocation = &mut self.allocations[view.allocation_id];

            allocation.ref_count -= 1;
            if allocation.ref_count == 0 {
                tracing::debug!(allocation_id = ?view.allocation_id, "removing allocation");

                self.allocator.deallocate(allocation.alloc_id);
                self.allocations.remove(view.allocation_id);
            }
        }
    }

    #[profiling::function]
    fn allocate(
        &mut self,
        size: Vector2<u32>,
        padding: Padding,
        change_index: Option<usize>,
    ) -> Result<(AllocationId, ViewId), Error> {
        self.handle_drops();

        // check if image won't ever fit
        if size.x > self.size_limit || size.y > self.size_limit {
            return Err(Error::TooLarge {
                image_size: size,
                limit: self.size_limit,
            });
        }

        let allocation_size = size + padding.additional_size();

        // allocate space for image
        let (allocation_offset, alloc_id) = loop {
            if let Some(allocation) = self
                .allocator
                .allocate(vector2_to_guillotiere(allocation_size))
            {
                let allocation_offset = guillotiere_to_vector2(allocation.rectangle.min);
                let max = guillotiere_to_vector2(allocation.rectangle.max);

                assert_eq!(max - allocation_offset, allocation_size);

                break (allocation_offset, allocation.id);
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

        let inner_offset = padding.inner_offset();

        let allocation_id = self.allocations.push(Allocation {
            alloc_id,
            pending_change: change_index,
            outer_offset: allocation_offset,
            outer_size: allocation_size,
            inner_offset: allocation_offset + inner_offset,
            inner_size: size,
            ref_count: 1,
        });

        let view_id = self.views.push(View {
            allocation_id,
            offset: inner_offset,
            size,
        });

        Ok((allocation_id, view_id))
    }

    #[profiling::function]
    pub fn insert_texture(
        &mut self,
        texture_view: wgpu::TextureView,
        padding_mode: Option<PaddingMode>,
    ) -> Result<AtlasHandle, Error> {
        let texture_size = texture_view.texture().size();
        let texture_size = Vector2::new(texture_size.width, texture_size.height);

        let change_index = self.changes.len();

        let (allocation_id, view_id) = self.allocate(
            texture_size,
            padding_mode
                .map(|padding_mode| padding_mode.padding)
                .unwrap_or_default(),
            Some(change_index),
        )?;

        self.changes.push(Change::Insert {
            allocation_id,
            source_texture: texture_view,
            source_offset: Vector2::zeros(),
            source_size: texture_size,
            padding_mode,
        });

        Ok(AtlasHandle {
            view_id,
            dropper: Arc::new(Dropper {
                view_id,
                dropped: self.dropped.clone(),
            }),
        })
    }

    #[profiling::function]
    pub fn insert_image(
        &mut self,
        image: &RgbaImage,
        padding_mode: Option<PaddingMode>,
        device: &wgpu::Device,
        staging: &mut Staging,
    ) -> Result<AtlasHandle, Error> {
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

        self.insert_texture(texture_view, padding_mode)
    }

    pub fn view(
        &mut self,
        handle: &AtlasHandle,
        offset: Vector2<u32>,
        size: Vector2<u32>,
    ) -> AtlasHandle {
        let outer_view = &self.views[handle.view_id];

        assert!(offset.x + size.x <= outer_view.size.x);
        assert!(offset.y + size.y <= outer_view.size.y);

        self.allocations[outer_view.allocation_id].ref_count += 1;

        let view_id = self.views.push(View {
            allocation_id: outer_view.allocation_id,
            offset: outer_view.offset + offset,
            size,
        });

        AtlasHandle {
            view_id,
            dropper: Arc::new(Dropper {
                view_id,
                dropped: self.dropped.clone(),
            }),
        }
    }

    pub fn view_size(&self, handle: &AtlasHandle) -> Vector2<u32> {
        self.views[handle.view_id].size
    }

    #[profiling::function]
    pub fn flush(&mut self, device: &wgpu::Device, mut staging: &mut Staging) -> bool {
        self.handle_drops();

        let mut new_texture = false;
        let new_data_buffer;

        // note: we might potentially want to change this check when we implement atlas
        // rearranging or some other changes
        if self.changes.is_empty() {
            // if there aren't any changes, early exit
            return false;
        }

        tracing::debug!("flushing texture atlas");

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

                blitter.blit_all_entries(
                    &mut self.changes,
                    &mut self.allocations,
                    &self.atlas_texture,
                );
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

                blitter.blit_changes(&self.changes, &mut self.allocations);
                blitter.finish(device, &mut staging);
            }
        }

        // update data buffer
        {
            let atlas_size_inv = 1.0 / (self.size as f32);

            new_data_buffer = self.data_buffer.write_all_with(
                self.views.len(),
                |buffer: &mut [DataBufferItem]| {
                    for (buffer_entry, view) in buffer
                        .iter_mut()
                        .zip_eq(self.views.iter().map(|(_index, allocation)| allocation))
                    {
                        let allocation = &self.allocations[view.allocation_id];

                        *buffer_entry = DataBufferItem {
                            uv_offset: atlas_size_inv
                                * (allocation.outer_offset + view.offset).cast::<f32>(),
                            uv_size: atlas_size_inv * view.size.cast::<f32>(),
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
                .map_buffer_on_submit(&staging_buffer.clone(), wgpu::MapMode::Read, .., move |result| {
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

impl Drop for Atlas {
    fn drop(&mut self) {
        let mut dropped = self.dropped.lock();
        dropped.closed = true;
        dropped.views.clear();
    }
}

#[derive(Debug, Default)]
struct Dropped {
    views: Vec<ViewId>,
    closed: bool,
}

#[derive(Clone)]
pub struct AtlasHandle {
    view_id: ViewId,
    #[allow(unused)]
    dropper: Arc<Dropper>,
}

impl AtlasHandle {
    pub fn id(&self) -> u32 {
        self.view_id.0.try_into().unwrap()
    }
}

impl Debug for AtlasHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("AtlasHandle").field(&self.view_id.0).finish()
    }
}

struct Dropper {
    view_id: ViewId,
    dropped: Arc<Mutex<Dropped>>,
}

impl Drop for Dropper {
    fn drop(&mut self) {
        let mut dropped = self.dropped.lock();
        if !dropped.closed {
            dropped.views.push(self.view_id);
        }
    }
}

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, derive_more::From, derive_more::Into,
)]
struct ViewId(usize);

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, derive_more::From, derive_more::Into,
)]
struct AllocationId(usize);

#[derive(Clone, Copy, Debug)]
pub struct AtlasResources<'a> {
    pub texture: &'a wgpu::TextureView,
    pub data_buffer: &'a wgpu::Buffer,
    pub version: AtlasVersion,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct AtlasVersion(usize);

#[derive(Clone, Copy, Debug)]
struct Allocation {
    alloc_id: guillotiere::AllocId,
    pending_change: Option<usize>,

    /// Offset in the atlas texture with padding
    outer_offset: Vector2<u32>,

    /// Size of allocation in the atlas texture with padding
    outer_size: Vector2<u32>,

    /// Offset in the atlas texture without padding
    inner_offset: Vector2<u32>,

    /// Size of allocation in the atlas texture without padding
    inner_size: Vector2<u32>,

    /// How many views reference this allocation
    ref_count: usize,
}

#[derive(Clone, Copy, Debug)]
struct View {
    allocation_id: AllocationId,

    /// offset relative to the allocation
    offset: Vector2<u32>,

    size: Vector2<u32>,
}

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
struct DataBufferItem {
    uv_offset: Vector2<f32>,
    uv_size: Vector2<f32>,
}

#[derive(Debug)]
enum Change {
    Insert {
        allocation_id: AllocationId,
        source_texture: wgpu::TextureView,
        source_offset: Vector2<u32>,
        source_size: Vector2<u32>,
        padding_mode: Option<PaddingMode>,
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
    pub const REPEAT: Self = Self::both(wgpu::AddressMode::Repeat);

    pub const fn both(address_mode: wgpu::AddressMode) -> Self {
        Self {
            address_mode_u: address_mode,
            address_mode_v: address_mode,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PaddingMode {
    pub padding: Padding,
    pub fill: PaddingFill,
}

#[derive(Clone, Copy, Debug)]
pub enum PaddingFill {
    Color { color: LinSrgba<f32> },
    Sampler { sampler_mode: SamplerMode },
}

impl PaddingFill {
    pub const REPEAT: Self = Self::Sampler {
        sampler_mode: SamplerMode::REPEAT,
    };

    pub const TRANSPARENT: Self = Self::Color {
        color: LinSrgba::new(0.0, 0.0, 0.0, 0.0),
    };
}

fn vector2_to_guillotiere(size: Vector2<u32>) -> guillotiere::Size {
    guillotiere::Size::new(
        i32::try_from(size.x).unwrap(),
        i32::try_from(size.y).unwrap(),
    )
}

fn guillotiere_to_vector2(point: guillotiere::Point) -> Vector2<u32> {
    Vector2::new(point.x.try_into().unwrap(), point.y.try_into().unwrap())
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
    fn keep(&mut self, old_atlas_texture: &wgpu::TextureView, allocation: &Allocation) {
        let sampler = get_sampler(self.samplers, self.device, SamplerMode::RESIZE);

        self.inner.blit(
            old_atlas_texture,
            sampler,
            allocation.outer_offset.cast::<i32>().into(),
            allocation.outer_size,
            allocation.outer_offset.cast::<i32>().into(),
            allocation.outer_size,
        );
    }

    fn insert(
        &mut self,
        source_texture: &wgpu::TextureView,
        source_offset: Point2<u32>,
        source_size: Vector2<u32>,
        padding_mode: Option<PaddingMode>,
        allocation: Allocation,
    ) {
        let mut sampler_mode = SamplerMode::RESIZE;
        let mut source_offset = source_offset.cast::<i32>();
        let mut source_size = source_size;
        let mut target_offset = allocation.inner_offset;
        let mut target_size = allocation.inner_size;

        if let Some(padding_mode) = padding_mode {
            match padding_mode.fill {
                PaddingFill::Color { color } => {
                    self.inner.fill(
                        color,
                        allocation.outer_offset.cast().into(),
                        allocation.outer_size,
                    );
                }
                PaddingFill::Sampler {
                    sampler_mode: sampler,
                } => {
                    sampler_mode = sampler;
                    source_offset -= padding_mode.padding.inner_offset().cast::<i32>();
                    source_size += padding_mode.padding.additional_size();
                    target_offset = allocation.outer_offset;
                    target_size = allocation.outer_size;
                }
            }
        }

        let source_sampler = get_sampler(self.samplers, self.device, sampler_mode);

        self.inner.blit(
            source_texture,
            source_sampler,
            source_offset,
            source_size,
            target_offset.cast().into(),
            target_size,
        );
    }

    fn blit_change<'e>(
        &mut self,
        change: &Change,
        mut get_allocation: impl FnMut(AllocationId) -> Allocation,
    ) {
        match change {
            Change::Insert {
                allocation_id,
                source_texture,
                source_offset,
                source_size,
                padding_mode,
            } => {
                let allocation = get_allocation(*allocation_id);
                self.insert(
                    source_texture,
                    (*source_offset).into(),
                    *source_size,
                    *padding_mode,
                    allocation,
                );
            }
        }
    }

    fn blit_changes(
        &mut self,
        changes: &[Change],
        allocations: &mut SparseVec<AllocationId, Allocation>,
    ) {
        for change in changes {
            self.blit_change(change, |allocation_id| {
                let allocation = &mut allocations[allocation_id];
                allocation.pending_change = None;
                *allocation
            })
        }
    }

    fn blit_all_entries(
        &mut self,
        changes: &mut [Change],
        entries: &mut SparseVec<AllocationId, Allocation>,
        old_altas_texture: &wgpu::TextureView,
    ) {
        for (_, allocation) in entries.iter_mut() {
            if let Some(change_index) = allocation.pending_change.take() {
                let change = &changes[change_index];
                self.blit_change(change, |_allocation_id| *allocation);
            }
            else {
                self.keep(old_altas_texture, allocation);
            }
        }
    }

    pub fn finish(self, device: &wgpu::Device, staging: &mut Staging) {
        self.inner.finish(device, staging);
    }
}
