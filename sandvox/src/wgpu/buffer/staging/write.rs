use std::{
    borrow::{
        Borrow,
        BorrowMut,
        Cow,
    },
    ops::Deref,
    sync::Arc,
};

use parking_lot::RwLock;

use self::inflight::*;
use crate::wgpu::TextureSourceLayout;

/// Trait for things that can be used to stage writes.
///
/// This is implemented on [`WriteStagingTransaction`], but allows naming types
/// easier (e.g. as `impl WriteStaging`) when only staging operations are
/// required. This has a blanket implementation for `&mut T` when `T` implements
/// the trait.
///
/// This is also implemented by [`CommitOnDrop`], which wraps any
/// [`WriteStaging`] (with [`WriteStagingCommit`]) and commits the transaction
/// when dropped.
pub trait WriteStaging {
    #[must_use]
    fn view_mut(
        &mut self,
        size: wgpu::BufferSize,
        alignment: wgpu::BufferSize,
        with_buffer_slice: impl FnOnce(&mut wgpu::CommandEncoder, wgpu::BufferSlice),
    ) -> wgpu::BufferViewMut;

    #[must_use]
    fn write_buffer(&mut self, destination: wgpu::BufferSlice) -> wgpu::BufferViewMut {
        let offset = destination.offset();
        let size = destination.size();

        assert!(
            size.get().is_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT),
            "allocation size {size} must be a multiple of `COPY_BUFFER_ALIGNMENT`"
        );
        assert!(
            offset.is_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT),
            "WriteStaging offset {offset} must be a multiple of `COPY_BUFFER_ALIGNMENT`"
        );

        self.view_mut(
            size,
            wgpu::BufferSize::new(wgpu::COPY_BUFFER_ALIGNMENT).unwrap(),
            |command_encoder, staging_buffer_slice| {
                command_encoder.copy_buffer_to_buffer(
                    staging_buffer_slice.buffer(),
                    staging_buffer_slice.offset(),
                    destination.buffer(),
                    offset,
                    size.get(),
                );
            },
        )
    }

    fn write_buffer_from_slice(&mut self, destination: wgpu::BufferSlice, data: &[u8]) {
        assert_eq!(destination.size().get(), data.len() as wgpu::BufferAddress);
        let mut view = self.write_buffer(destination);
        view.copy_from_slice(data);
    }

    #[must_use]
    fn write_texture(
        &mut self,
        source_layout: TextureSourceLayout,
        destination: wgpu::TexelCopyTextureInfo,
        size: wgpu::Extent3d,
    ) -> wgpu::BufferViewMut {
        assert!(
            source_layout
                .bytes_per_row
                .is_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT),
            "Bytes per row does not respect `COPY_BYTES_PER_ROW_ALIGNMENT`"
        );

        //tracing::debug!(?source_layout, ?destination, ?size, "copy to texture");

        let mut copy_size = wgpu::BufferAddress::from(size.height)
            * wgpu::BufferAddress::from(source_layout.bytes_per_row);
        if size.depth_or_array_layers > 1 {
            let rows_per_image = source_layout.rows_per_image.expect("`rows_per_image` must be specified when copying with a size that has `depth_or_array_layers` > 1");
            copy_size *= wgpu::BufferAddress::from(size.depth_or_array_layers)
                * wgpu::BufferAddress::from(rows_per_image);
        }
        let copy_size = wgpu::BufferSize::new(copy_size).expect("Texture size must not be zero");

        // todo: multiple of texture block size
        let alignment = wgpu::BufferSize::new(wgpu::COPY_BUFFER_ALIGNMENT).unwrap();

        self.view_mut(
            copy_size,
            alignment,
            |command_encoder, staging_buffer_slice| {
                command_encoder.copy_buffer_to_texture(
                    source_layout.into_texel_copy_buffer_info(staging_buffer_slice),
                    destination,
                    size,
                );
            },
        )
    }
}

impl<T> WriteStaging for &mut T
where
    T: WriteStaging,
{
    fn view_mut(
        &mut self,
        size: wgpu::BufferSize,
        alignment: wgpu::BufferSize,
        with_buffer_slice: impl FnOnce(&mut wgpu::CommandEncoder, wgpu::BufferSlice),
    ) -> wgpu::BufferViewMut {
        T::view_mut(self, size, alignment, with_buffer_slice)
    }
}

pub trait WriteStagingCommit {
    type CommitResult;
    type DiscardResult;

    /// Commits the staging transaction
    fn commit(self) -> Self::CommitResult;

    /// Discard the staging transaction.
    ///
    /// This is usually the default behavior (it really depends on the
    /// [`StagingBufferProvider`] implementation), but calling it implicitely
    /// might return some resources that are held by the transaction.
    fn discard(self) -> Self::DiscardResult;
}

pub trait WriteStagingExt: WriteStaging {
    fn track_throughput<'a, 'b>(
        &'a mut self,
        bytes: &'b mut wgpu::BufferAddress,
    ) -> TrackThroughput<'b, &'a mut Self> {
        TrackThroughput { inner: self, bytes }
    }
}

impl<T> WriteStagingExt for T where T: WriteStaging {}

/// A generic implementation of [`WriteStagingTransaction`].
///
/// It is generic over the staging buffer provider, device and command encoder.
/// Any staging buffer provider can be used, but it must always be passed as
/// owned. The device can be either owned or borrowed. The command encoder can
/// be either owned or mut-borrowed. This is such that device and command
/// encoder can be passed as owned or borrowed, depending on the usecase. Access
/// to these is always provided, since their fields are `pub`.
///
/// # TODO
///
/// The buffer provider could probably be `P: BorrowMut<P>` too, since the trait
/// allows reuse of these.
#[derive(Debug)]
pub struct WriteStagingTransaction<Provider, Device, Encoder> {
    provider: Provider,
    pub device: Device,
    pub command_encoder: Encoder,
}

impl<Provider, Device, Encoder> WriteStagingTransaction<Provider, Device, Encoder>
where
    Provider: StagingBufferProvider,
    Device: Borrow<wgpu::Device>,
    Encoder: BorrowMut<wgpu::CommandEncoder>,
{
    pub fn new(provider: Provider, device: Device, command_encoder: Encoder) -> Self {
        Self {
            provider,
            device,
            command_encoder,
        }
    }
}

impl<Provider, Device, Encoder> WriteStaging for WriteStagingTransaction<Provider, Device, Encoder>
where
    Provider: StagingBufferProvider,
    Device: Borrow<wgpu::Device>,
    Encoder: BorrowMut<wgpu::CommandEncoder>,
{
    fn view_mut(
        &mut self,
        size: wgpu::BufferSize,
        alignment: wgpu::BufferSize,
        with_buffer_slice: impl FnOnce(&mut wgpu::CommandEncoder, wgpu::BufferSlice),
    ) -> wgpu::BufferViewMut {
        assert!(
            size.get().is_multiple_of(wgpu::COPY_BUFFER_ALIGNMENT),
            "WriteStagingBelt allocation size {size} must be a multiple of `COPY_BUFFER_ALIGNMENT`"
        );
        assert!(
            alignment.get().is_power_of_two(),
            "alignment must be a power of two, not {alignment}"
        );

        // At minimum, we must have alignment sufficient to map the buffer.
        let alignment = alignment.max(wgpu::BufferSize::new(wgpu::MAP_ALIGNMENT).unwrap());

        self.provider.allocate(
            self.device.borrow(),
            size,
            alignment,
            |staging_buffer_slice| {
                with_buffer_slice(self.command_encoder.borrow_mut(), staging_buffer_slice);
                staging_buffer_slice.get_mapped_range_mut()
            },
        )
    }
}

impl<Provider, Device, Encoder> WriteStagingCommit
    for WriteStagingTransaction<Provider, Device, Encoder>
where
    Provider: StagingBufferProvider,
    Encoder: BorrowMut<wgpu::CommandEncoder>,
{
    type CommitResult = Encoder;
    type DiscardResult = ();

    fn commit(mut self) -> Self::CommitResult {
        // tell the staging buffer provider to finish everything it has to do
        // (i.e. register callbacks with the command encoder to unmap the buffers when
        // the commands finish executing)
        self.provider.commit(self.command_encoder.borrow_mut());
        self.command_encoder
    }

    fn discard(mut self) -> Self::DiscardResult {
        self.provider.discard();
    }
}

/// Wraps a `WriteStaging + WriteStagingCommit` and commits it when dropped.
///
/// In order to fully commit the transaction command buffer will need to be
/// submitted to the queue. Thus the wrapped transaction must return the command
/// buffer as a commit result, and you must provide the queue.
#[derive(Debug)]
pub struct SubmitOnDrop<Transaction, Queue>
where
    Transaction: WriteStaging + WriteStagingCommit,
    wgpu::CommandEncoder: From<Transaction::CommitResult>,
    Queue: Borrow<wgpu::Queue>,
{
    inner: Option<(Transaction, Queue)>,
}

impl<Transaction, Queue> SubmitOnDrop<Transaction, Queue>
where
    Transaction: WriteStaging + WriteStagingCommit,
    wgpu::CommandEncoder: From<Transaction::CommitResult>,
    Queue: Borrow<wgpu::Queue>,
{
    pub fn new(transaction: Transaction, queue: Queue) -> Self {
        Self {
            inner: Some((transaction, queue)),
        }
    }

    pub fn inner(&self) -> &Transaction {
        // this is only None in Drop
        &self.inner.as_ref().unwrap().0
    }

    pub fn inner_mut(&mut self) -> &mut Transaction {
        // this is only None in Drop
        &mut self.inner.as_mut().unwrap().0
    }

    pub fn into_parts(mut self) -> (Transaction, Queue) {
        self.inner.take().unwrap()
    }
}

impl<Transaction, Queue> WriteStaging for SubmitOnDrop<Transaction, Queue>
where
    Transaction: WriteStaging + WriteStagingCommit,
    wgpu::CommandEncoder: From<Transaction::CommitResult>,
    Queue: Borrow<wgpu::Queue>,
{
    fn view_mut(
        &mut self,
        size: wgpu::BufferSize,
        alignment: wgpu::BufferSize,
        with_buffer_slice: impl FnOnce(&mut wgpu::CommandEncoder, wgpu::BufferSlice),
    ) -> wgpu::BufferViewMut {
        self.inner_mut()
            .view_mut(size, alignment, with_buffer_slice)
    }
}

impl<Transaction, Queue> WriteStagingCommit for SubmitOnDrop<Transaction, Queue>
where
    Transaction: WriteStaging + WriteStagingCommit,
    wgpu::CommandEncoder: From<Transaction::CommitResult>,
    Queue: Borrow<wgpu::Queue>,
{
    type CommitResult = ();
    type DiscardResult = Transaction::DiscardResult;

    fn commit(self) -> Self::CommitResult {
        let (transaction, queue) = self.into_parts();
        let command_buffer: wgpu::CommandEncoder = transaction.commit().into();
        queue.borrow().submit([command_buffer.finish()]);
    }

    fn discard(self) -> Self::DiscardResult {
        let (transaction, _queue) = self.into_parts();
        transaction.discard()
    }
}

impl<Transaction, Queue> Drop for SubmitOnDrop<Transaction, Queue>
where
    Transaction: WriteStaging + WriteStagingCommit,
    wgpu::CommandEncoder: From<Transaction::CommitResult>,
    Queue: Borrow<wgpu::Queue>,
{
    fn drop(&mut self) {
        if let Some((transaction, queue)) = self.inner.take() {
            let command_buffer: wgpu::CommandEncoder = transaction.commit().into();
            queue.borrow().submit([command_buffer.finish()]);
        }
    }
}

#[derive(Debug)]
pub struct TrackThroughput<'a, Transaction> {
    pub inner: Transaction,
    pub bytes: &'a mut wgpu::BufferAddress,
}

impl<'a, Transaction> WriteStaging for TrackThroughput<'a, Transaction>
where
    Transaction: WriteStaging,
{
    fn view_mut(
        &mut self,
        size: wgpu::BufferSize,
        alignment: wgpu::BufferSize,
        with_buffer_slice: impl FnOnce(&mut wgpu::CommandEncoder, wgpu::BufferSlice),
    ) -> wgpu::BufferViewMut {
        *self.bytes += size.get();
        self.inner.view_mut(size, alignment, with_buffer_slice)
    }
}

impl<'a, Transaction> WriteStagingCommit for TrackThroughput<'a, Transaction>
where
    Transaction: WriteStagingCommit,
{
    type CommitResult = Transaction::CommitResult;
    type DiscardResult = Transaction::DiscardResult;

    fn commit(self) -> Self::CommitResult {
        self.inner.commit()
    }

    fn discard(self) -> Self::DiscardResult {
        self.inner.discard()
    }
}

pub trait StagingBufferProvider {
    fn allocate<R>(
        &mut self,
        device: &wgpu::Device,
        size: wgpu::BufferSize,
        alignment: wgpu::BufferSize,
        f: impl FnOnce(wgpu::BufferSlice<'_>) -> R,
    ) -> R;

    fn commit(&mut self, command_encoder: &mut wgpu::CommandEncoder);
    fn discard(&mut self);
}

#[derive(Clone, Debug, Default)]
pub struct OneShotStaging {
    active_buffers: Vec<wgpu::Buffer>,
}

impl StagingBufferProvider for OneShotStaging {
    fn allocate<R>(
        &mut self,
        device: &wgpu::Device,
        size: wgpu::BufferSize,
        alignment: wgpu::BufferSize,
        f: impl FnOnce(wgpu::BufferSlice<'_>) -> R,
    ) -> R {
        let _ = alignment;
        let staging_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("one-time write staging"),
            size: size.get(),
            usage: wgpu::BufferUsages::COPY_SRC | wgpu::BufferUsages::MAP_WRITE,
            mapped_at_creation: true,
        });

        let output = f(staging_buffer.slice(..));

        self.active_buffers.push(staging_buffer);

        output
    }

    fn commit(&mut self, command_encoder: &mut wgpu::CommandEncoder) {
        let inflight_buffers = std::mem::take(&mut self.active_buffers);

        command_encoder.on_submitted_work_done(move || {
            for buffer in inflight_buffers {
                buffer.unmap();
            }
        });
    }

    fn discard(&mut self) {
        self.active_buffers.clear();
    }
}

#[derive(Clone, Debug)]
pub struct StagingPool {
    inner: Arc<StagingPoolInner>,
}

#[derive(Debug)]
struct StagingPoolInner {
    /// Minimum size of an individual chunk
    chunk_size: wgpu::BufferSize,
    chunk_label: Cow<'static, str>,
    state: RwLock<StagingPoolState>,
}

#[derive(Debug, Default)]
struct StagingPoolState {
    /// Chunks that are back from the GPU and ready to be mapped for write and
    /// put into `active_chunks`.
    free_chunks: Vec<Chunk>,
    in_flight_count: usize,
    total_allocated_count: usize,
    total_allocated_bytes: u64,
    total_staged_bytes: u64,
}

impl Default for StagingPool {
    fn default() -> Self {
        Self::new(wgpu::BufferSize::new(0x1000).unwrap(), "staging pool")
    }
}

impl StagingPool {
    pub fn new(chunk_size: wgpu::BufferSize, chunk_label: impl Into<Cow<'static, str>>) -> Self {
        Self {
            inner: Arc::new(StagingPoolInner {
                chunk_size,
                chunk_label: chunk_label.into(),
                state: RwLock::new(Default::default()),
            }),
        }
    }

    #[must_use]
    pub fn belt(&self) -> WriteStagingBelt {
        WriteStagingBelt::from_pool(self.clone())
    }

    pub fn info(&self) -> StagingPoolInfo {
        let state = self.inner.state.read();
        StagingPoolInfo {
            in_flight_count: state.in_flight_count,
            free_count: state.free_chunks.len(),
            total_allocation_count: state.total_allocated_count,
            total_allocation_bytes: state.total_allocated_bytes,
            total_staged_bytes: state.total_staged_bytes,
        }
    }
}

#[derive(Debug)]
pub struct WriteStagingBelt {
    pool: StagingPool,

    /// Chunks into which we are accumulating data to be transferred.
    ///
    /// Note: if the WriteStagingBelt is dropped while it has active chunks
    /// (i.e. finish wasn't called), the chunks will not be reused.
    active_chunks: Vec<Chunk>,
}

impl WriteStagingBelt {
    pub fn new(chunk_size: wgpu::BufferSize, chunk_label: impl Into<Cow<'static, str>>) -> Self {
        Self::from_pool(StagingPool::new(chunk_size, chunk_label))
    }

    pub fn from_pool(pool: StagingPool) -> Self {
        Self {
            pool,
            active_chunks: vec![],
        }
    }

    fn discard_impl(&mut self) {
        let mut state = self.pool.inner.state.write();
        state.in_flight_count -= self.active_chunks.len();
        state
            .free_chunks
            .extend(self.active_chunks.drain(..).map(|mut chunk| {
                chunk.reset();
                chunk
            }));
    }
}

impl StagingBufferProvider for WriteStagingBelt {
    fn allocate<R>(
        &mut self,
        device: &wgpu::Device,
        size: wgpu::BufferSize,
        alignment: wgpu::BufferSize,
        f: impl FnOnce(wgpu::BufferSlice<'_>) -> R,
    ) -> R {
        let chunk_index = self
            .active_chunks
            .iter()
            .position(|chunk| chunk.can_allocate(size, alignment.get()))
            .unwrap_or_else(|| {
                let mut state = self.pool.inner.state.write();
                state.in_flight_count += 1;

                let chunk = if let Some(index) = state
                    .free_chunks
                    .iter()
                    .position(|chunk| chunk.can_allocate(size, alignment.get()))
                {
                    state.free_chunks.swap_remove(index)
                }
                else {
                    let size = self.pool.inner.chunk_size.get().max(size.get());
                    state.total_allocated_count += 1;
                    state.total_allocated_bytes += size;
                    drop(state);

                    Chunk {
                        buffer: device.create_buffer(&wgpu::BufferDescriptor {
                            label: Some(&self.pool.inner.chunk_label),
                            size,
                            usage: wgpu::BufferUsages::MAP_WRITE | wgpu::BufferUsages::COPY_SRC,
                            mapped_at_creation: true,
                        }),
                        offset: 0,
                    }
                };

                let chunk_index = self.active_chunks.len();
                self.active_chunks.push(chunk);
                chunk_index
            });

        let chunk = &mut self.active_chunks[chunk_index];
        let allocation_offset = chunk.allocate(size, alignment.get());

        let staging_buffer_slice = chunk
            .buffer
            .slice(allocation_offset..allocation_offset + size.get());

        f(staging_buffer_slice)
    }

    fn commit(&mut self, command_encoder: &mut wgpu::CommandEncoder) {
        for chunk in &self.active_chunks {
            chunk.buffer.unmap();
        }

        let inflight_chunks =
            InflightChunks::new(self.pool.clone(), std::mem::take(&mut self.active_chunks));

        command_encoder.on_submitted_work_done(move || {
            // the command encoder got submitted and is done, we can recall the chunks
            inflight_chunks.recall();
        });
    }

    fn discard(&mut self) {
        if !self.active_chunks.is_empty() {
            self.discard_impl()
        }
    }
}

impl Drop for WriteStagingBelt {
    fn drop(&mut self) {
        if !self.active_chunks.is_empty() {
            tracing::warn!("WriteStagingBelt not committed. Staging buffers will not be mapped.");
            self.discard_impl();
        }
    }
}

/// Helpers to make sure in-flight chunks are always accounted for.
///
/// This basically wraps them and handles the case if they're dropped somewhere.
mod inflight {
    use super::*;

    // when we recall the chunks and map them, we need to move them individually
    // into the map_async callback with a pool anyway. so we pair them up
    // now.
    pub(super) struct InflightChunk {
        inner: Option<(StagingPool, Chunk)>,
    }

    // then we give them a Drop impl to make sure they're always accounted for
    impl Drop for InflightChunk {
        fn drop(&mut self) {
            if let Some((pool, chunk)) = self.inner.take() {
                // this chunk got lost somewhere (map_sync dropped it). we'll drop it because we
                // don't know its state (whether it's mapped or not). but we want to take it
                // into account
                tracing::warn!(?chunk, "inflight chunk dropped");
                let mut state = pool.inner.state.write();
                state.in_flight_count -= 1;
            }
        }
    }

    impl Deref for InflightChunk {
        type Target = Chunk;

        fn deref(&self) -> &Self::Target {
            // this is always okay, because we only take out the chunk when we take
            // ownership of this.
            &self.inner.as_ref().unwrap().1
        }
    }

    impl InflightChunk {
        pub fn new(pool: StagingPool, chunk: Chunk) -> Self {
            Self {
                inner: Some((pool, chunk)),
            }
        }
        pub fn into_inner(mut self) -> (StagingPool, Chunk) {
            self.inner.take().unwrap()
        }
    }

    // this will hold all the inflight chunks for the on_submitted_work_done
    // callback. if the user drops the command encoder this will be dropped,
    // and we can safely recall the chunks
    pub(super) struct InflightChunks {
        pool: StagingPool,
        chunks: Vec<Chunk>,
    }

    impl InflightChunks {
        pub fn new(pool: StagingPool, chunks: Vec<Chunk>) -> Self {
            Self { pool, chunks }
        }
    }

    impl InflightChunks {
        pub fn recall(mut self) {
            // we could just drop it, since the drop impl will call the same method, but
            // this is more explicit.
            self.recall_impl();
        }

        fn recall_impl(&mut self) {
            for chunk in self.chunks.drain(..) {
                let buffer = chunk.buffer.clone();

                let chunk = InflightChunk::new(self.pool.clone(), chunk);

                buffer.map_async(wgpu::MapMode::Write, .., move |result| {
                    if let Err(error) = result {
                        tracing::error!("{error}");
                    }
                    else {
                        // take out the chunk from the `InflightChunk`, so it's Drop doesn't do
                        // anything
                        let (pool, mut chunk) = chunk.into_inner();

                        // well, this includes alignment, but it's only used for debug info :shrug:
                        let allocated = chunk.offset;

                        chunk.reset();

                        // take account and put back into free list
                        let mut state = pool.inner.state.write();
                        state.in_flight_count -= 1;
                        state.total_staged_bytes += allocated;
                        state.free_chunks.push(chunk);
                    }
                });
            }
        }
    }

    impl Drop for InflightChunks {
        fn drop(&mut self) {
            // this is to make sure active buffers are recalled even if the command encoder
            // is dropped and never submitted
            self.recall_impl();
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct StagingPoolInfo {
    pub in_flight_count: usize,
    pub free_count: usize,
    pub total_allocation_count: usize,
    pub total_allocation_bytes: u64,
    pub total_staged_bytes: u64,
}

#[derive(Debug)]
struct Chunk {
    buffer: wgpu::Buffer,
    offset: wgpu::BufferAddress,
}

impl Chunk {
    fn can_allocate(&self, size: wgpu::BufferSize, alignment: wgpu::BufferAddress) -> bool {
        let alloc_start = wgpu::util::align_to(self.offset, alignment);
        let alloc_end = alloc_start + size.get();

        alloc_end <= self.buffer.size()
    }

    fn allocate(
        &mut self,
        size: wgpu::BufferSize,
        alignment: wgpu::BufferAddress,
    ) -> wgpu::BufferAddress {
        let alloc_start = wgpu::util::align_to(self.offset, alignment);
        let alloc_end = alloc_start + size.get();

        assert!(alloc_end <= self.buffer.size());
        self.offset = alloc_end;
        alloc_start
    }

    fn reset(&mut self) {
        self.offset = 0;
    }
}
