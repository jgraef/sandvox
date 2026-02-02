pub mod context;
pub mod main_pass;
pub mod phase;
pub mod ui_pass;

use crate::profiler::wgpu::{
    RenderPassProfiler,
    SpanId,
};

#[derive(Debug, derive_more::Deref, derive_more::DerefMut)]
pub struct RenderPass<'a> {
    #[deref]
    #[deref_mut]
    render_pass: wgpu::RenderPass<'a>,
    profiler: Option<RenderPassProfiler>,
}

impl<'a> RenderPass<'a> {
    #[track_caller]
    #[inline]
    pub fn enter_span(&mut self, label: &'static str) -> Span {
        if let Some(profiler) = &mut self.profiler {
            Span(Some(profiler.enter_span(label, &mut self.render_pass)))
        }
        else {
            Span(None)
        }
    }

    #[track_caller]
    #[inline]
    pub fn exit_span(&mut self, span: Span) {
        if let (Some(profiler), Some(span_id)) = (&mut self.profiler, span.0) {
            profiler.exit_span(span_id, &mut self.render_pass);
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Span(Option<SpanId>);
