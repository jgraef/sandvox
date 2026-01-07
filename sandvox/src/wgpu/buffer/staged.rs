use std::borrow::Cow;

use bytemuck::Pod;

use crate::wgpu::buffer::{
    TypedArrayBuffer,
    WriteStaging,
};

/// A [`TypedArrayBuffer`] with a staging buffer (a simple [`Vec`]).
#[derive(Debug)]
pub struct StagedTypedArrayBuffer<T> {
    pub buffer: TypedArrayBuffer<T>,
    pub host_staging: Vec<T>,
}

impl<T> StagedTypedArrayBuffer<T> {
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
        initial_capacity: usize,
    ) -> Self {
        let buffer = TypedArrayBuffer::with_capacity(
            device,
            label,
            usage | wgpu::BufferUsages::COPY_DST,
            initial_capacity,
        );
        Self::from_buffer(buffer)
    }

    pub fn from_buffer(buffer: TypedArrayBuffer<T>) -> Self {
        assert!(
            buffer.usage().contains(wgpu::BufferUsages::COPY_DST),
            "Buffer must contain BufferUsages::COPY_DST to be used as a staged buffer."
        );
        Self {
            buffer,
            host_staging: vec![],
        }
    }

    pub fn push(&mut self, item: T) {
        self.host_staging.push(item);
    }
}

impl<T> StagedTypedArrayBuffer<T>
where
    T: Pod,
{
    pub fn from_data(
        device: wgpu::Device,
        label: impl Into<Cow<'static, str>>,
        usage: wgpu::BufferUsages,
        data: Vec<T>,
    ) -> Self {
        let buffer = TypedArrayBuffer::from_slice(
            device,
            label,
            usage | wgpu::BufferUsages::COPY_DST,
            &data,
        );
        Self {
            buffer,
            host_staging: data,
        }
    }

    /// Flushes the staged data to the GPU.
    ///
    /// Returns `true` if the buffer was reallocated.
    pub fn flush<S>(&mut self, on_reallocate: impl FnMut(&wgpu::Buffer), gpu_staging: S) -> bool
    where
        S: WriteStaging,
    {
        if self.host_staging.is_empty() {
            // the below code works fine for an empty instance buffer, and it'll basically
            // do nothing, but we can still exit early.
            return false;
        }

        let reallocated = self
            .buffer
            .write_all(&self.host_staging, on_reallocate, gpu_staging);

        self.host_staging.clear();

        reallocated
    }
}
