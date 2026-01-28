use std::{
    panic::Location,
    sync::mpsc,
};

use crate::{
    profiler::Profiler,
    wgpu::query::{
        QuerySetAllocation,
        QuerySetPool,
        QuerySetTransaction,
        ResolvedQuerySetTransaction,
    },
};

#[derive(Clone, Debug)]
pub struct WgpuProfiler {
    pool: QuerySetPool,
    sink: WgpuProfilerSink,
}

impl WgpuProfiler {
    pub fn new(device: &wgpu::Device, timestamp_period: f32, profiler: &Profiler) -> Self {
        let pool = QuerySetPool::new(device, wgpu::QueryType::Timestamp, "profiler");
        let sink = profiler.wgpu_sink(timestamp_period);

        Self { pool, sink }
    }

    #[track_caller]
    pub fn begin_render_pass(&self, label: &'static str) -> RenderPassProfiler {
        let render_pass_caller = Location::caller();

        let transaction = self.pool.begin();

        RenderPassProfiler {
            transaction,
            start_end: None,
            sink: self.sink.clone(),
            render_pass_caller,
            spans: vec![],
            label,
        }
    }
}

/// # Bugs
///
/// Due to a [bug][1] in wgpu you **must** use the allocated query slots.
///
/// [1]: https://github.com/gfx-rs/wgpu/issues/7238
#[derive(Debug)]
pub struct RenderPassProfiler {
    transaction: QuerySetTransaction,
    start_end: Option<QuerySetAllocation>,
    sink: WgpuProfilerSink,
    render_pass_caller: &'static Location<'static>,
    spans: Vec<QuerySpan>,
    label: &'static str,
}

impl RenderPassProfiler {
    pub fn timestamp_writes(&mut self) -> wgpu::RenderPassTimestampWrites<'_> {
        let start_end = self
            .start_end
            .get_or_insert_with(|| self.transaction.allocate(2));

        wgpu::RenderPassTimestampWrites {
            query_set: self.transaction.get_query_set(*start_end),
            beginning_of_pass_write_index: Some(start_end.first_query_index),
            end_of_pass_write_index: Some(start_end.first_query_index + 1),
        }
    }

    #[track_caller]
    pub fn enter_span(
        &mut self,
        label: &'static str,
        render_pass: &mut wgpu::RenderPass,
    ) -> SpanId {
        let caller = Location::caller();

        let query = self.transaction.allocate(1);

        let span_id = SpanId(self.spans.len());
        self.spans.push(QuerySpan {
            label,
            enter: QueryEvent {
                caller,
                query,
                timestamp: 0,
            },
            exit: None,
        });

        render_pass.write_timestamp(
            self.transaction.get_query_set(query),
            query.first_query_index,
        );

        span_id
    }

    #[track_caller]
    pub fn exit_span(&mut self, span_id: SpanId, render_pass: &mut wgpu::RenderPass) {
        let caller = Location::caller();

        let span = &mut self.spans[span_id.0];

        let query = self.transaction.allocate(1);

        render_pass.write_timestamp(
            self.transaction.get_query_set(query),
            query.first_query_index,
        );

        span.exit = Some(QueryEvent {
            caller,
            query,
            timestamp: 0,
        });
    }

    pub fn finish(mut self, command_encoder: &mut wgpu::CommandEncoder) {
        if let Some(start_end) = self.start_end {
            self.transaction.finish(command_encoder, move |resolved| {
                let reference_time = get_reference_timestamp();

                let render_pass_times: [u64; 2] = {
                    let data = resolved.get(start_end);
                    let timestamps: &[u64] = bytemuck::cast_slice(data);
                    timestamps.try_into().unwrap()
                };

                for span in &mut self.spans {
                    span.enter.resolve(&resolved);
                    if let Some(exit) = &mut span.exit {
                        exit.resolve(&resolved);
                    }
                }

                self.sink.write(
                    reference_time,
                    RenderPassSpan {
                        label: self.label,
                        caller: self.render_pass_caller,
                        start: render_pass_times[0],
                        end: render_pass_times[1],
                    },
                    self.spans,
                );
            });
        }
    }
}

#[inline(always)]
fn get_reference_timestamp() -> i64 {
    #![allow(unused)]

    // todo: a nicer way of abstracting the reference time needed by the backend

    let mut t = 0;

    #[cfg(feature = "puffin")]
    {
        t = puffin::now_ns();
    }

    t
}

#[derive(Clone, Debug, Default)]
pub struct WgpuProfilerSink {
    sender: Option<mpsc::SyncSender<WriterCommand>>,
}

impl WgpuProfilerSink {
    fn write(&self, reference_time: i64, render_pass: RenderPassSpan, spans: Vec<QuerySpan>) {
        if let Some(sender) = &self.sender {
            let _ = sender.try_send(WriterCommand::Write {
                reference_time,
                render_pass,
                spans,
            });
        }
    }
}

#[allow(dead_code)]
#[derive(Debug)]
struct QueryEvent {
    caller: &'static Location<'static>,
    query: QuerySetAllocation,
    timestamp: u64,
}

impl QueryEvent {
    #[inline]
    fn resolve(&mut self, resolved: &ResolvedQuerySetTransaction) {
        let data = resolved.get(self.query);
        self.timestamp = *bytemuck::from_bytes(data);
    }
}

#[allow(dead_code)]
#[derive(Debug)]
struct QuerySpan {
    label: &'static str,
    enter: QueryEvent,
    exit: Option<QueryEvent>,
}

#[allow(dead_code)]
#[derive(Debug)]
struct RenderPassSpan {
    label: &'static str,
    caller: &'static Location<'static>,
    start: u64,
    end: u64,
}

#[derive(Clone, Copy, Debug)]
pub struct SpanId(usize);

#[allow(dead_code)]
enum WriterCommand {
    Write {
        reference_time: i64,
        render_pass: RenderPassSpan,
        spans: Vec<QuerySpan>,
    },
}

#[cfg(feature = "puffin")]
pub mod puffin_sink {
    use std::{
        collections::HashMap,
        panic::Location,
        sync::mpsc,
    };

    use color_eyre::eyre::Error;
    use puffin::{
        GlobalProfiler,
        ScopeDetails,
        ScopeId,
        StreamInfo,
        ThreadInfo,
    };

    use crate::profiler::wgpu::{
        WgpuProfilerSink,
        WriterCommand,
    };

    #[inline]
    fn location_to_details(
        location: &'static Location<'static>,
        label: &'static str,
    ) -> ScopeDetails {
        ScopeDetails::from_scope_name(label)
            .with_file(location.file())
            .with_line_nr(location.line())
    }

    pub fn create_sink(timestamp_period: f32) -> WgpuProfilerSink {
        let (sender, receiver) = mpsc::sync_channel(0x1000);

        std::thread::Builder::new()
            .name("wgpu-puffin-sink".into())
            .spawn(move || writer_thread(receiver, timestamp_period))
            .unwrap();

        WgpuProfilerSink {
            sender: Some(sender),
        }
    }

    fn writer_thread(
        receiver: mpsc::Receiver<WriterCommand>,
        timestamp_period: f32,
    ) -> Result<(), Error> {
        let span = tracing::info_span!("puffin-sink-thread");
        let _guard = span.enter();

        let mut scopes: HashMap<&'static Location<'static>, ScopeId> = HashMap::new();
        let mut details = vec![];

        while let Ok(command) = receiver.recv() {
            match command {
                WriterCommand::Write {
                    reference_time,
                    render_pass,
                    spans,
                } => {
                    let mut profiler = GlobalProfiler::lock();

                    let mut stream_info = StreamInfo::default();

                    // register new scopes
                    assert!(details.is_empty());
                    if !scopes.contains_key(render_pass.caller) {
                        details.push(location_to_details(render_pass.caller, render_pass.label));
                    }
                    for span in &spans {
                        if !scopes.contains_key(span.enter.caller) {
                            details.push(location_to_details(span.enter.caller, span.label));
                        }
                    }

                    let mut scope_ids = profiler.register_user_scopes(&details).into_iter();

                    // insert registered scope ids
                    if !scopes.contains_key(render_pass.caller) {
                        scopes.insert(render_pass.caller, scope_ids.next().unwrap());
                    }
                    for (span, scope_id) in spans.iter().zip(scope_ids) {
                        if !scopes.contains_key(span.enter.caller) {
                            details.push(location_to_details(span.enter.caller, span.label));
                            scopes.insert(span.enter.caller, scope_id);
                        }
                    }

                    // returns scope id for location
                    let scope_id =
                        |location: &'static Location<'static>| *scopes.get(location).unwrap();

                    // returns nanoseconds since start of render pass
                    let to_ns = |timestamp| {
                        //start_ns
                        //    + ((timestamp - render_pass.start) as f32 * timestamp_period) as i64
                        reference_time
                            - ((render_pass.end - timestamp) as f32 * timestamp_period) as i64
                    };

                    // serialize spans
                    let render_pass_start_ns = to_ns(render_pass.start);
                    let render_pass_end_ns = to_ns(render_pass.end);
                    let (offset, _) = stream_info.stream.begin_scope(
                        || render_pass_start_ns,
                        scope_id(render_pass.caller),
                        "",
                    );
                    stream_info.num_scopes += 1;
                    stream_info.depth = 1;

                    for span in &spans {
                        if let Some(exit) = &span.exit {
                            let (offset, _) = stream_info.stream.begin_scope(
                                || to_ns(span.enter.timestamp),
                                scope_id(span.enter.caller),
                                "",
                            );

                            stream_info.stream.end_scope(offset, to_ns(exit.timestamp));
                            stream_info.num_scopes += 1;
                            stream_info.depth = 2;
                        }
                    }

                    stream_info.stream.end_scope(offset, render_pass_end_ns);
                    stream_info.range_ns = (render_pass_start_ns, render_pass_end_ns);

                    // submit stream
                    profiler.report_user_scopes(
                        ThreadInfo {
                            start_time_ns: None,
                            name: "gpu".to_owned(),
                        },
                        &stream_info.as_stream_into_ref(),
                    );

                    details.clear();
                }
            }
        }

        Ok(())
    }
}
