use std::ops::{
    Deref,
    DerefMut,
};

use bevy_ecs::{
    change_detection::DetectChanges,
    resource::Resource,
    system::{
        Deferred,
        Res,
        ResMut,
        SystemBuffer,
        SystemMeta,
        SystemParam,
    },
    world::World,
};

use crate::{
    profiler::wgpu::{
        RenderPassProfiler,
        SpanId,
    },
    render::staging::Staging,
    wgpu::WgpuContext,
};

#[derive(derive_more::Debug, SystemParam)]
pub struct RenderContext<'w, 's> {
    wgpu: Res<'w, WgpuContext>,
    #[debug(skip)]
    state: Deferred<'s, State>,
    // todo: move staging transation into this
}

impl<'w, 's> RenderContext<'w, 's> {
    pub fn flush(&mut self) {
        self.state.flush();
    }

    pub fn command_encoder(&mut self) -> &mut wgpu::CommandEncoder {
        self.state.command_encoder(&self.wgpu.device)
    }

    #[track_caller]
    pub fn begin_render_pass<'a>(
        &'a mut self,
        descriptor: &wgpu::RenderPassDescriptor,
        label: &'static str,
    ) -> RenderPass<'a> {
        // this is a bit awkward to do
        let (render_pass, profiler, command_encoder) = if descriptor.timestamp_writes.is_none()
            && let Some(profiler) = &self.wgpu.profiler
        {
            let mut profiler = profiler.begin_render_pass(label);
            let descriptor = wgpu::RenderPassDescriptor {
                timestamp_writes: Some(profiler.timestamp_writes()),
                // we think the clone here is fine
                ..descriptor.clone()
            };

            let command_encoder = self.command_encoder();
            let render_pass = command_encoder
                .begin_render_pass(&descriptor)
                .forget_lifetime();

            (render_pass, Some(profiler), command_encoder)
        }
        else {
            let command_encoder = self.command_encoder();
            let render_pass = command_encoder
                .begin_render_pass(&descriptor)
                .forget_lifetime();

            (render_pass, None, command_encoder)
        };

        RenderPass {
            render_pass: Some(render_pass),
            command_encoder,
            profiler,
        }
    }
}

#[derive(Debug, Default)]
struct State {
    command_encoder: Option<wgpu::CommandEncoder>,
    command_buffers: Vec<wgpu::CommandBuffer>,
}

impl State {
    fn command_encoder(&mut self, device: &wgpu::Device) -> &mut wgpu::CommandEncoder {
        self.command_encoder.get_or_insert_with(|| {
            device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("render context"),
            })
        })
    }

    fn flush(&mut self) {
        if let Some(command_encoder) = self.command_encoder.take() {
            self.command_buffers.push(command_encoder.finish());
        }
    }
}

impl SystemBuffer for State {
    fn apply(&mut self, system_meta: &SystemMeta, world: &mut World) {
        let _ = system_meta;

        self.flush();

        let mut pending = world.resource_mut::<PendingCommandBuffers>();
        pending
            .command_buffers
            .extend(self.command_buffers.drain(..))
    }
}

#[derive(Debug, Default, Resource)]
pub struct PendingCommandBuffers {
    command_buffers: Vec<wgpu::CommandBuffer>,
}

pub fn flush_command_buffers(
    wgpu: Res<WgpuContext>,
    mut pending: ResMut<PendingCommandBuffers>,
    mut staging: ResMut<Staging>,
) {
    // we want all the staged transfers to happen first
    //
    // todo: how does queue ordering work exactly?
    let command_buffers = staging.is_changed().then(|| staging.flush(&wgpu).finish());

    // then take all other pending command buffers
    let command_buffers = command_buffers
        .into_iter()
        .chain(pending.command_buffers.drain(..));

    // and submit everything
    wgpu.queue.submit(command_buffers);
}

#[derive(Debug)]
pub struct RenderPass<'a> {
    // note: we need to make this a 'static lifetime, so we can pass the command encoder alongside,
    // so that we can finish the profiler
    render_pass: Option<wgpu::RenderPass<'static>>,
    command_encoder: &'a mut wgpu::CommandEncoder,
    profiler: Option<RenderPassProfiler>,
}

impl<'a> RenderPass<'a> {
    #[track_caller]
    #[inline]
    pub fn enter_span(&mut self, label: &'static str) -> Span {
        if let Some(profiler) = &mut self.profiler {
            Span(Some(
                profiler.enter_span(label, self.render_pass.as_mut().unwrap()),
            ))
        }
        else {
            Span(None)
        }
    }

    #[track_caller]
    #[inline]
    pub fn exit_span(&mut self, span: Span) {
        if let (Some(profiler), Some(span_id)) = (&mut self.profiler, span.0) {
            profiler.exit_span(span_id, self.render_pass.as_mut().unwrap());
        }
    }
}

impl<'a> Deref for RenderPass<'a> {
    type Target = wgpu::RenderPass<'static>;

    fn deref(&self) -> &Self::Target {
        self.render_pass.as_ref().unwrap()
    }
}

impl<'a> DerefMut for RenderPass<'a> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.render_pass.as_mut().unwrap()
    }
}

impl<'a> Drop for RenderPass<'a> {
    fn drop(&mut self) {
        // we must make sure that the render pass is dropped first
        let _ = self.render_pass.take();

        if let Some(profiler) = self.profiler.take() {
            profiler.finish(self.command_encoder);
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Span(Option<SpanId>);
