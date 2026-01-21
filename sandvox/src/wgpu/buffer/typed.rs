use std::{
    borrow::Cow,
    marker::PhantomData,
    ops::{
        Deref,
        DerefMut,
        Range,
        RangeBounds,
    },
    sync::Arc,
};

use bytemuck::Pod;

use crate::{
    util::{
        normalize_index_bounds,
        oneshot,
    },
    wgpu::buffer::WriteStaging,
};

// note: this is intentionally not Clone
#[derive(Debug)]
pub struct TypedArrayBuffer<T> {
    inner: Option<TypedArrayBufferInner>,
    device: wgpu::Device,
    label: Cow<'static, str>,
    usage: wgpu::BufferUsages,
    _phantom: PhantomData<[T]>,
}

impl<T> TypedArrayBuffer<T> {
    fn new_impl(
        device: wgpu::Device,
        label: Cow<'static, str>,
        capacity: usize,
        num_elements: usize,
        usage: wgpu::BufferUsages,
        mapped_at_creation: bool,
    ) -> Self {
        // todo: do we want to store if its mapped (and if it's read/write)?

        let mut buffer = Self {
            inner: None,
            device: device.clone(),
            label,
            usage,
            _phantom: PhantomData,
        };

        buffer.allocate_inner(capacity, num_elements, mapped_at_creation);

        buffer
    }

    fn allocate_inner(
        &mut self,
        capacity: usize,
        num_elements: usize,
        mapped_at_creation: bool,
    ) -> Option<TypedArrayBufferInner> {
        let old_inner = self.inner.take();

        assert!(capacity >= num_elements);

        if capacity != 0 {
            self.inner = Some(TypedArrayBufferInner::new::<T>(
                &self.device,
                &self.label,
                capacity,
                num_elements,
                self.usage,
                mapped_at_creation,
            ));
        }

        old_inner
    }

    pub fn new(
        device: wgpu::Device,
        label: impl Into<Cow<'static, str>>,
        usage: wgpu::BufferUsages,
    ) -> Self {
        Self::with_capacity(device, label, usage, 0)
    }

    pub fn with_capacity(
        device: wgpu::Device,
        label: impl Into<Cow<'static, str>>,
        usage: wgpu::BufferUsages,
        capacity: usize,
    ) -> Self {
        Self::new_impl(device, label.into(), capacity, 0, usage, false)
    }

    /// Returns a reference to the underlying [`wgpu::Buffer`].
    ///
    /// # Panics
    ///
    /// Panics if the buffer has size 0 and thus has no underlying buffer. Use
    /// [`try_buffer`] if you want to handle this case yourself.
    pub fn buffer(&self) -> &wgpu::Buffer {
        self.try_buffer().expect("Zero-sized buffer")
    }

    pub fn try_buffer(&self) -> Option<&wgpu::Buffer> {
        self.inner.as_ref().map(|inner| &inner.buffer)
    }

    pub fn len(&self) -> usize {
        self.inner.as_ref().map_or(0, |inner| inner.num_elements)
    }

    pub fn capacity(&self) -> usize {
        self.inner.as_ref().map_or(0, |inner| inner.capacity)
    }

    pub fn is_empty(&self) -> bool {
        self.inner
            .as_ref()
            .is_none_or(|inner| inner.num_elements == 0)
    }

    pub fn usage(&self) -> wgpu::BufferUsages {
        self.usage
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn is_allocated(&self) -> bool {
        self.inner.is_some()
    }
}

impl<T> TypedArrayBuffer<T>
where
    T: Pod,
{
    pub fn from_slice(
        device: wgpu::Device,
        label: impl Into<Cow<'static, str>>,
        usage: wgpu::BufferUsages,
        data: &[T],
    ) -> Self {
        Self::from_fn_with_view(device, label, data.len(), usage, |view| {
            view.copy_from_slice(data);
        })
    }

    pub fn from_value(
        device: wgpu::Device,
        label: impl Into<Cow<'static, str>>,
        num_elements: usize,
        usage: wgpu::BufferUsages,
        value: T,
    ) -> Self {
        Self::from_fn_with_view(device, label, num_elements, usage, |view| {
            view.fill(value);
        })
    }

    pub fn from_fn(
        device: wgpu::Device,
        label: impl Into<Cow<'static, str>>,
        num_elements: usize,
        usage: wgpu::BufferUsages,
        mut fill: impl FnMut(usize) -> T,
    ) -> Self {
        Self::from_fn_with_view(device, label, num_elements, usage, |view| {
            view.iter_mut()
                .enumerate()
                .for_each(|(index, value)| *value = fill(index));
        })
    }

    pub fn from_fn_with_view(
        device: wgpu::Device,
        label: impl Into<Cow<'static, str>>,
        num_elements: usize,
        usage: wgpu::BufferUsages,
        mut fill: impl FnMut(&mut [T]),
    ) -> Self {
        let mut buffer = Self::new_impl(
            device,
            label.into(),
            num_elements,
            num_elements,
            usage,
            true,
        );

        if let Some(inner) = &mut buffer.inner {
            inner.with_mapped_mut(|view| fill(&mut view[..num_elements]))
        }
        else {
            fill(&mut []);
        }

        buffer
    }

    pub fn read_view<'a>(
        &'a self,
        range: impl RangeBounds<usize>,
        queue: &wgpu::Queue,
    ) -> TypedArrayBufferReadView<'a, T> {
        self.inner
            .as_ref()
            .and_then(|inner| {
                let index_range = normalize_index_bounds(range, inner.num_elements);
                (!index_range.is_empty())
                    .then(|| TypedArrayBufferReadView::new(index_range, inner, &self.device, queue))
            })
            .unwrap_or(TypedArrayBufferReadView {
                inner: None,
                _phantom: PhantomData,
            })
    }

    pub fn write_view<'buffer, S>(
        &'buffer mut self,
        range: impl RangeBounds<usize>,
        staging: S,
    ) -> TypedArrayBufferWriteView<'buffer, T>
    where
        S: WriteStaging,
    {
        self.inner
            .as_mut()
            .and_then(|inner| {
                let index_range = normalize_index_bounds(range, inner.num_elements);

                (!index_range.is_empty())
                    .then(|| TypedArrayBufferWriteView::new(index_range, inner, staging))
            })
            .unwrap_or(TypedArrayBufferWriteView {
                inner: None,
                _phantom: PhantomData,
            })
    }

    /// Reallocates the buffer for a larger size.
    ///
    /// This only actually reallocates if the current capacity is less than
    /// `new_elements`.
    ///
    /// If a closure is passed as `on_reallocate`, it will be called with:
    ///
    /// 1. a mapped slice of the old data, if `pass_old_view` is `true` **and**
    ///    the buffer supports [`wgpu::BufferUsages::COPY_SRC`]
    /// 2. a mapped mut-slice of the new buffer. This is always present, as it's
    ///    cheap to map the buffer on creation.
    /// 3. the new [`wgpu::Buffer`]
    ///
    /// With this it's possible to copy data from the old buffer to the new one,
    /// if desired. The new underlying [`wgpu::Buffer`] can also be used to
    /// recreate any bind groups if necessary.
    ///
    /// This returns `true` if an reallocation did take place.
    pub fn reallocate_for_size<F>(
        &mut self,
        num_elements: usize,
        mut on_reallocate: Option<F>,
        read_from_old_buffer: Option<&wgpu::Queue>,
    ) -> bool
    where
        F: FnMut(Option<&[T]>, &mut [T], &wgpu::Buffer),
    {
        let current_capacity = self.capacity();

        if num_elements > current_capacity {
            // todo: make this a generic parameter?
            let new_capacity = (current_capacity * 2).max(num_elements);

            let old_inner =
                self.allocate_inner(new_capacity, num_elements, on_reallocate.is_some());

            if let Some(on_reallocate) = &mut on_reallocate {
                let can_read = self.usage.contains(wgpu::BufferUsages::COPY_SRC);

                let old_view = read_from_old_buffer.and_then(|queue| {
                    can_read
                        .then(|| {
                            old_inner.as_ref().map(|inner| {
                                TypedArrayBufferReadView::new(
                                    0..inner.num_elements,
                                    inner,
                                    &self.device,
                                    queue,
                                )
                            })
                        })
                        .flatten()
                });

                let new_inner = self
                    .inner
                    .as_ref()
                    .expect("we just reallocated with larger capacity");

                // note: this unmaps the buffer
                new_inner.with_mapped_mut(|new_view| {
                    on_reallocate(
                        old_view.as_deref(),
                        &mut new_view[..num_elements],
                        &new_inner.buffer,
                    );
                });
            }

            true
        }
        else {
            false
        }
    }

    pub fn write_all<S>(
        &mut self,
        data: &[T],
        on_reallocate: impl FnMut(&wgpu::Buffer),
        staging: S,
    ) -> bool
    where
        S: WriteStaging,
    {
        self.write_all_with(
            data.len(),
            |view| view.copy_from_slice(data),
            on_reallocate,
            staging,
        )
    }

    pub fn write_all_with<S>(
        &mut self,
        size: usize,
        mut write: impl FnMut(&mut [T]),
        mut on_reallocate: impl FnMut(&wgpu::Buffer),
        staging: S,
    ) -> bool
    where
        S: WriteStaging,
    {
        let did_reallocate = self.reallocate_for_size(
            size,
            Some(
                |_old_view: Option<&[T]>, new_view: &mut [T], new_buffer: &wgpu::Buffer| {
                    write(new_view);

                    on_reallocate(new_buffer);
                },
            ),
            None,
        );

        if !did_reallocate {
            // still need to write the data
            let mut view = self.write_view(..size, staging);
            write(&mut *view);
        }

        did_reallocate
    }
}

#[derive(Debug)]
struct TypedArrayBufferInner {
    buffer: wgpu::Buffer,
    num_elements: usize,
    capacity: usize,
    unpadded_buffer_size: wgpu::BufferAddress,
}

impl TypedArrayBufferInner {
    fn new<T>(
        device: &wgpu::Device,
        label: &str,
        capacity: usize,
        num_elements: usize,
        usage: wgpu::BufferUsages,
        mapped_at_creation: bool,
    ) -> Self {
        assert!(capacity > 0);

        let unpadded_buffer_size = unpadded_buffer_size::<T>(capacity);
        let padded_buffer_size = if mapped_at_creation || buffer_usage_needs_padding(usage) {
            pad_buffer_size_for_copy(unpadded_buffer_size)
        }
        else {
            unpadded_buffer_size
        };

        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: padded_buffer_size,
            usage,
            mapped_at_creation,
        });

        Self {
            buffer,
            num_elements,
            capacity,
            unpadded_buffer_size,
        }
    }

    /// This is for when you mapped the buffer at creation
    fn with_mapped_mut<T, R>(&self, mut f: impl FnMut(&mut [T]) -> R) -> R
    where
        T: Pod,
    {
        let mut view = self.buffer.get_mapped_range_mut(..);
        let view_slice: &mut [T] =
            bytemuck::cast_slice_mut(&mut view[..(self.unpadded_buffer_size as usize)]);
        let output = f(view_slice);
        drop(view);
        self.buffer.unmap();
        output
    }
}

// note: don't make this Clone. While it would be nice to have, the Drop impl
// then needs to take into account if there are more outstanding mapped view,
// e.g. by adding a reference count. At this point the user can just Arc the
// whole view.
#[derive(Debug)]
pub struct TypedArrayBufferReadView<'a, T> {
    inner: Option<TypedBufferReadViewInner>,
    _phantom: PhantomData<&'a [T]>,
}

impl<'a, T> TypedArrayBufferReadView<'a, T> {
    fn new(
        index_range: Range<usize>,
        inner: &'a TypedArrayBufferInner,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
    ) -> Self {
        let alignment =
            StagingBufferAlignment::from_unaligned_buffer_range_typed::<T>(index_range.clone());

        let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("read: staging"),
            size: alignment.copy_size.get(),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });

        let mut command_encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("read: copy to staging"),
        });

        command_encoder.copy_buffer_to_buffer(
            &inner.buffer,
            alignment.buffer_start,
            &staging_buffer,
            0,
            alignment.copy_size.get(),
        );

        let (result_sender, result_receiver) = oneshot::channel();

        command_encoder.map_buffer_on_submit(
            &staging_buffer,
            wgpu::MapMode::Read,
            ..,
            move |result| {
                let _ = result_sender.send(result);
            },
        );

        let submission_index = queue.submit([command_encoder.finish()]);

        device
            .poll(wgpu::PollType::Wait {
                submission_index: Some(submission_index),
                timeout: None,
            })
            .expect("device poll failed");

        result_receiver
            .receive()
            .expect("Device didn't call our map-buffer-callback")
            .expect("map_buffer_on_submit failed");

        let staging_view = staging_buffer.get_mapped_range(..);

        Self {
            inner: Some(TypedBufferReadViewInner {
                alignment,
                staging_buffer,
                staging_view: Arc::new(staging_view),
            }),
            _phantom: PhantomData,
        }
    }
}

impl<'a, T> AsRef<[T]> for TypedArrayBufferReadView<'a, T>
where
    T: Pod,
{
    fn as_ref(&self) -> &[T] {
        self.inner
            .as_ref()
            .map(|inner| bytemuck::cast_slice(&inner.staging_view[inner.alignment.staging_range()]))
            .unwrap_or(&[])
    }
}

impl<'a, T> Deref for TypedArrayBufferReadView<'a, T>
where
    T: Pod,
{
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl<'a, T> Drop for TypedArrayBufferReadView<'a, T> {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.take() {
            drop(inner.staging_view);
            inner.staging_buffer.unmap();
        }
    }
}

#[derive(Debug)]
struct TypedBufferReadViewInner {
    alignment: StagingBufferAlignment,
    staging_buffer: wgpu::Buffer,
    staging_view: Arc<wgpu::BufferView>,
}

#[derive(Debug)]
pub struct TypedArrayBufferWriteView<'buffer, T> {
    inner: Option<TypedArrayBufferWriteViewInner>,
    _phantom: PhantomData<&'buffer mut [T]>,
}

impl<'buffer, T> TypedArrayBufferWriteView<'buffer, T> {
    fn new<S>(
        index_range: Range<usize>,
        inner: &'buffer TypedArrayBufferInner,
        mut staging: S,
    ) -> Self
    where
        S: WriteStaging,
    {
        let alignment =
            StagingBufferAlignment::from_unaligned_buffer_range_typed::<T>(index_range.clone());

        // this is just nasty to fix and we could make it a hard requirement anyway.
        #[allow(clippy::todo)]
        if !alignment.is_aligned() {
            todo!("unaligned write");
        }

        let target = inner
            .buffer
            .slice(alignment.buffer_start..alignment.buffer_end);
        let staging_view = staging.write_buffer(target);

        Self {
            inner: Some(TypedArrayBufferWriteViewInner {
                alignment,
                staging_view,
            }),
            _phantom: PhantomData,
        }
    }
}

impl<'buffer, T> AsRef<[T]> for TypedArrayBufferWriteView<'buffer, T>
where
    T: Pod,
{
    fn as_ref(&self) -> &[T] {
        self.inner
            .as_ref()
            .map(|inner| bytemuck::cast_slice(&inner.staging_view[inner.alignment.staging_range()]))
            .unwrap_or(&[])
    }
}

impl<'buffer, T> AsMut<[T]> for TypedArrayBufferWriteView<'buffer, T>
where
    T: Pod,
{
    fn as_mut(&mut self) -> &mut [T] {
        self.inner
            .as_mut()
            .map(|inner| {
                bytemuck::cast_slice_mut(&mut inner.staging_view[inner.alignment.staging_range()])
            })
            .unwrap_or(&mut [])
    }
}

impl<'buffer, T> Deref for TypedArrayBufferWriteView<'buffer, T>
where
    T: Pod,
{
    type Target = [T];

    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl<'buffer, T> DerefMut for TypedArrayBufferWriteView<'buffer, T>
where
    T: Pod,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.as_mut()
    }
}

#[derive(Debug)]
struct TypedArrayBufferWriteViewInner {
    alignment: StagingBufferAlignment,
    staging_view: wgpu::BufferViewMut,
}

#[derive(Clone, Copy, Debug)]
struct StagingBufferAlignment {
    pub buffer_start: wgpu::BufferAddress,
    pub buffer_end: wgpu::BufferAddress,
    pub staging_start: wgpu::BufferAddress,
    pub staging_end: wgpu::BufferAddress,
    pub copy_size: wgpu::BufferSize,
}

impl StagingBufferAlignment {
    pub fn from_unaligned_buffer_range_typed<T>(index_range: Range<usize>) -> Self {
        let unaligned_copy_source = Range {
            start: (size_of::<T>() * index_range.start) as wgpu::BufferAddress,
            end: (size_of::<T>() * index_range.end) as wgpu::BufferAddress,
        };
        Self::from_unaligned_buffer_range(unaligned_copy_source)
    }

    pub fn from_unaligned_buffer_range(unaligned_buffer_range: Range<wgpu::BufferAddress>) -> Self {
        let unaligned_copy_size = unaligned_buffer_range.end - unaligned_buffer_range.start;

        let buffer_start = align_copy_start_offset(unaligned_buffer_range.start);
        let copy_size = pad_buffer_size_for_copy(unaligned_copy_size);
        let buffer_end = buffer_start + copy_size;

        let staging_start = unaligned_buffer_range.start - buffer_start;
        let staging_end = unaligned_buffer_range.end - buffer_start;

        Self {
            buffer_start,
            buffer_end,
            staging_start,
            staging_end,
            copy_size: wgpu::BufferSize::new(copy_size).expect("copy size is zero"),
        }
    }

    pub fn staging_range(&self) -> Range<usize> {
        (self.staging_start as usize)..(self.staging_end as usize)
    }

    pub fn is_aligned(&self) -> bool {
        self.staging_start == 0 && self.staging_end == self.copy_size.get()
    }
}

pub fn unpadded_buffer_size<T>(num_elements: usize) -> wgpu::BufferAddress {
    (size_of::<T>() * num_elements) as wgpu::BufferAddress
}

pub const BUFFER_COPY_ALIGN_MASK: wgpu::BufferAddress = wgpu::COPY_BUFFER_ALIGNMENT - 1;

pub fn align_copy_start_offset(offset: wgpu::BufferAddress) -> wgpu::BufferAddress {
    offset & !BUFFER_COPY_ALIGN_MASK
}

pub fn pad_buffer_size_for_copy(unpadded_size: wgpu::BufferAddress) -> wgpu::BufferAddress {
    // https://github.com/gfx-rs/wgpu/blob/836c97056fb2c32852d1d8f6f45fefba1d1d6d26/wgpu/src/util/device.rs#L52
    // Valid vulkan usage is
    // 1. buffer size must be a multiple of COPY_BUFFER_ALIGNMENT.
    // 2. buffer size must be greater than 0.
    // Therefore we round the value up to the nearest multiple, and ensure it's at
    // least COPY_BUFFER_ALIGNMENT.
    ((unpadded_size + BUFFER_COPY_ALIGN_MASK) & !BUFFER_COPY_ALIGN_MASK)
        .max(wgpu::COPY_BUFFER_ALIGNMENT)
}

pub fn buffer_usage_needs_padding(usage: wgpu::BufferUsages) -> bool {
    // Not sure if MAP_READ or MAP_WRITE needs padding. copying definitely needs it,
    // since the documentation of copy_buffer_to_buffer states that copies need to
    // be multiples of COPY_BUFFER_ALIGNMENT.
    //
    // I checked [wgpu::util::DownladBuffer][1], but it doesn't even pad the size.
    //
    // [1]: https://github.com/gfx-rs/wgpu/blob/836c97056fb2c32852d1d8f6f45fefba1d1d6d26/wgpu/src/util/mod.rs#L166
    usage.intersects(wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC)
}

pub fn is_buffer_copy_aligned(index: wgpu::BufferAddress) -> bool {
    (index & BUFFER_COPY_ALIGN_MASK) == 0
}
