use std::sync::Arc;

use parking_lot::Mutex;

#[derive(Clone, Debug)]
pub struct QuerySetPool {
    shared: Arc<Shared>,
}

impl QuerySetPool {
    pub fn new(device: &wgpu::Device, ty: wgpu::QueryType, label: &str) -> Self {
        // see: https://docs.rs/wgpu/latest/wgpu/enum.QueryType.html
        let num_query_items = match ty {
            wgpu::QueryType::Occlusion => 1,
            wgpu::QueryType::PipelineStatistics(pipeline_statistics_types) => {
                assert!(
                    !pipeline_statistics_types.is_empty(),
                    "Empty PipelineStatisticsTypes"
                );
                pipeline_statistics_types.bits().count_ones()
            }
            wgpu::QueryType::Timestamp => 1,
        };
        let bytes_per_query = u64::from(num_query_items * wgpu::QUERY_SIZE);
        let default_capacity =
            u32::try_from(wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT / bytes_per_query).map_or_else(
                |_| wgpu::QUERY_SET_MAX_QUERIES,
                |value| value.min(wgpu::QUERY_SET_MAX_QUERIES),
            );

        Self {
            shared: Arc::new(Shared {
                default_capacity,
                ty,
                query_set_label: format!("{label} query set"),
                resolve_buffer_label: format!("{label} resolve buffer"),
                staging_buffer_label: format!("{label} staging buffer"),
                device: device.clone(),
                bytes_per_query,
                state: Default::default(),
            }),
        }
    }

    pub fn begin(&self) -> QuerySetTransaction {
        QuerySetTransaction {
            shared: self.shared.clone(),
            active_query_sets: vec![],
            first_non_full: 0,
        }
    }
}

#[derive(Debug)]
pub struct QuerySetTransaction {
    shared: Arc<Shared>,
    active_query_sets: Vec<ActiveQuerySet>,
    first_non_full: usize,
}

impl QuerySetTransaction {
    /// # Bugs
    ///
    /// Due to a [bug][1] in wgpu you **must** use the allocated query set slots
    /// if you wish to resolve them
    ///
    /// [1]: https://github.com/gfx-rs/wgpu/issues/7238
    pub fn allocate(&mut self, num_queries: u32) -> QuerySetAllocation {
        assert!(num_queries > 0);

        let query_set_index = 'found: {
            for index in self.first_non_full..self.active_query_sets.len() {
                let active_query_set = &self.active_query_sets[index];
                let remaining = active_query_set.query_set.capacity - active_query_set.used;

                if remaining == 0 {
                    self.first_non_full += 1;
                }
                else {
                    if num_queries <= remaining {
                        break 'found index;
                    }
                }
            }

            // didn't find a query set we can use so we need to allocate one
            let mut state = self.shared.state.lock();

            let query_set = if let Some(index) = state
                .free_query_sets
                .iter()
                .position(|query_set| num_queries < query_set.capacity)
            {
                state.num_active_query_sets += 1;
                state.free_query_sets.swap_remove(index)
            }
            else {
                let capacity = self.shared.default_capacity.max(num_queries);

                let query_set = self
                    .shared
                    .device
                    .create_query_set(&wgpu::QuerySetDescriptor {
                        label: Some(&self.shared.query_set_label),
                        ty: self.shared.ty,
                        count: capacity,
                    });

                state.num_active_query_sets += 1;
                state.num_allocated_query_sets += 1;

                tracing::debug!(
                    ?state.num_active_query_sets,
                    ?state.num_allocated_query_sets,
                    "allocated query set"
                );

                QuerySet {
                    query_set,
                    capacity,
                }
            };

            let index = self.active_query_sets.len();
            self.active_query_sets.push(ActiveQuerySet {
                query_set,
                used: 0,
                buffer_offset: 0,
            });

            index
        };

        let first_query_index = {
            let active_query_set = &mut self.active_query_sets[query_set_index];

            let first_query_index = active_query_set.used;
            active_query_set.used += num_queries;
            assert!(active_query_set.used <= active_query_set.query_set.capacity);

            first_query_index
        };

        QuerySetAllocation {
            query_set_index,
            first_query_index,
            num_queries,
        }
    }

    pub fn get_query_set<'a>(&'a self, allocation: QuerySetAllocation) -> &'a wgpu::QuerySet {
        &self.active_query_sets[allocation.query_set_index]
            .query_set
            .query_set
    }

    /// # Bugs
    ///
    /// Due to a [bug][1] in wgpu you must have used all allocated query set
    /// slots, i.e. they must have been written to. Otherwise resolving them is
    /// undefined behavior and might lead to resolution taking too long (~20s)
    /// and generally the application crashing.
    ///
    /// [1]: https://github.com/gfx-rs/wgpu/issues/7238
    pub fn finish<F>(mut self, command_encoder: &mut wgpu::CommandEncoder, callback: F)
    where
        F: FnOnce(ResolvedQuerySetTransaction) + Send + 'static,
    {
        if self.active_query_sets.is_empty() {
            // nothing to do
            return;
        }

        let buffer_size = {
            // calculate required size of resolve buffer

            let mut offset = 0;
            for active_query_set in &mut self.active_query_sets {
                offset = wgpu::util::align_to(offset, wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT);
                active_query_set.buffer_offset = offset;
                offset += u64::from(active_query_set.used) * self.shared.bytes_per_query;
            }

            offset
        };

        let resolve_buffer = {
            // allocate resolve buffer

            let mut state = self.shared.state.lock();

            if let Some(index) = state
                .free_resolve_buffers
                .iter()
                .position(|resolve_buffer| resolve_buffer.capacity.get() >= buffer_size)
            {
                state.num_active_resolve_buffers += 1;
                state.free_resolve_buffers.swap_remove(index)
            }
            else {
                let buffer_size =
                    wgpu::util::align_to(buffer_size, wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT);

                let resolve_buffer = self.shared.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(&self.shared.resolve_buffer_label),
                    size: buffer_size,
                    usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                    mapped_at_creation: false,
                });

                let staging_buffer = self.shared.device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some(&self.shared.staging_buffer_label),
                    size: buffer_size,
                    usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });

                state.num_allocated_resolve_buffers += 1;
                state.num_active_resolve_buffers += 1;
                state.total_resolve_buffer_memory += 2 * buffer_size;

                tracing::debug!(
                    ?buffer_size,
                    ?state.num_allocated_resolve_buffers,
                    ?state.num_active_resolve_buffers,
                    "allocated resolve buffer"
                );

                ResolveBuffer {
                    resolve_buffer,
                    staging_buffer,
                    capacity: {
                        // we checked that active_query_sets is not empty and
                        // bytes_per_query is always > 0
                        wgpu::BufferSize::new(buffer_size).unwrap()
                    },
                }
            }
        };

        // resolve queries into buffer
        for active_query_set in &self.active_query_sets {
            tracing::trace!(num_queries = active_query_set.used, "resolving queries");

            command_encoder.resolve_query_set(
                &active_query_set.query_set.query_set,
                0..active_query_set.used,
                &resolve_buffer.resolve_buffer,
                active_query_set.buffer_offset,
            );
        }

        // copy to staging buffer
        let copy_size = wgpu::util::align_to(buffer_size, wgpu::COPY_BUFFER_ALIGNMENT);
        command_encoder.copy_buffer_to_buffer(
            &resolve_buffer.resolve_buffer,
            0,
            &resolve_buffer.staging_buffer,
            0,
            copy_size,
        );

        // map staging buffer
        // todo: map resolve buffer such that we can handle drops
        command_encoder.map_buffer_on_submit(
            &resolve_buffer.staging_buffer.clone(),
            wgpu::MapMode::Read,
            ..buffer_size,
            move |result| {
                result.unwrap();

                let buffer = resolve_buffer
                    .staging_buffer
                    .get_mapped_range(..buffer_size);

                callback(ResolvedQuerySetTransaction {
                    inner: &self,
                    buffer: &buffer,
                });

                // unmap staging buffer
                drop(buffer);
                resolve_buffer.staging_buffer.unmap();

                {
                    // put buffer back into pool to be reused

                    let mut state = self.shared.state.lock();
                    state.free_resolve_buffers.push(resolve_buffer);
                    state.num_active_resolve_buffers -= 1;
                }
            },
        );
    }
}

impl Drop for QuerySetTransaction {
    fn drop(&mut self) {
        if !self.active_query_sets.is_empty() {
            let mut state = self.shared.state.lock();
            for active_query_set in self.active_query_sets.drain(..) {
                state.free_query_sets.push(active_query_set.query_set);
                state.num_active_query_sets -= 1;
            }
        }
    }
}

#[derive(Debug)]
pub struct ResolvedQuerySetTransaction<'a> {
    inner: &'a QuerySetTransaction,
    buffer: &'a [u8],
}

impl<'a> ResolvedQuerySetTransaction<'a> {
    pub fn get(&self, allocation: QuerySetAllocation) -> &[u8] {
        let active_query_set = &self.inner.active_query_sets[allocation.query_set_index];

        let start = usize::try_from(
            active_query_set.buffer_offset
                + u64::from(allocation.first_query_index) * self.inner.shared.bytes_per_query,
        )
        .unwrap();

        let end = start
            + usize::try_from(
                u64::from(allocation.num_queries) * self.inner.shared.bytes_per_query,
            )
            .unwrap();

        &self.buffer[start..end]
    }
}

#[derive(Clone, Copy, Debug)]
pub struct QuerySetAllocation {
    query_set_index: usize,
    pub first_query_index: u32,
    pub num_queries: u32,
}

#[derive(Debug)]
struct Shared {
    default_capacity: u32,
    ty: wgpu::QueryType,
    query_set_label: String,
    resolve_buffer_label: String,
    staging_buffer_label: String,
    device: wgpu::Device,
    bytes_per_query: u64,
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    free_query_sets: Vec<QuerySet>,
    num_active_query_sets: usize,
    num_allocated_query_sets: usize,
    free_resolve_buffers: Vec<ResolveBuffer>,
    num_active_resolve_buffers: usize,
    num_allocated_resolve_buffers: usize,
    total_resolve_buffer_memory: u64,
}

#[derive(Debug)]
struct QuerySet {
    query_set: wgpu::QuerySet,
    capacity: u32,
}

#[derive(Debug)]
struct ActiveQuerySet {
    query_set: QuerySet,
    used: u32,
    buffer_offset: u64,
}

#[derive(Debug)]
struct ResolveBuffer {
    resolve_buffer: wgpu::Buffer,
    staging_buffer: wgpu::Buffer,
    capacity: wgpu::BufferSize,
}
