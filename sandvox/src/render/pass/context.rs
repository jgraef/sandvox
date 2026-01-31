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
    render::{
        pass::RenderPass,
        staging::Staging,
    },
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

    pub fn begin_render_pass<'a>(
        &'a mut self,
        descriptor: &wgpu::RenderPassDescriptor,
    ) -> RenderPass<'a> {
        if descriptor.timestamp_writes.is_none()
            && let Some(profiler) = &self.wgpu.profiler
        {
            let mut profiler = profiler.begin_render_pass("todo");
            let descriptor = wgpu::RenderPassDescriptor {
                timestamp_writes: Some(profiler.timestamp_writes()),
                // we think the clone here is fine
                ..descriptor.clone()
            };

            let render_pass = self.command_encoder().begin_render_pass(&descriptor);

            RenderPass {
                render_pass,
                profiler: Some(profiler),
            }
        }
        else {
            let render_pass = self.command_encoder().begin_render_pass(&descriptor);

            RenderPass {
                render_pass,
                profiler: None,
            }
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
