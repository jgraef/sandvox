use bevy_ecs::{
    resource::Resource,
    system::{
        Commands,
        Res,
        ResMut,
    },
};

use crate::wgpu::{
    WgpuContext,
    buffer::{
        WriteStaging,
        WriteStagingBelt,
        WriteStagingCommit,
        WriteStagingTransaction,
    },
};

/// Initializes a staging transaction before any rendering setup systems run
pub(super) fn initialize_staging(wgpu: Res<WgpuContext>, mut commands: Commands) {
    commands.insert_resource(Staging::new(&wgpu));
}

/// Flushes the current staging transaction.
///
/// This is done in the setup schedule after rendering setup systems have run.
/// During rendering this is done in `end_frames`
#[profiling::function]
pub(super) fn flush_staging(wgpu: Res<WgpuContext>, mut staging: ResMut<Staging>) {
    wgpu.queue.submit([staging.flush(&wgpu).finish()]);
}

// rename to `RenderStaging`? we think this should be only used for rendering
// purposes
#[derive(Debug, Resource)]
pub struct Staging {
    staging_transaction:
        WriteStagingTransaction<WriteStagingBelt, wgpu::Device, wgpu::CommandEncoder>,
}

impl Staging {
    pub fn new(wgpu: &WgpuContext) -> Self {
        let command_encoder = wgpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("staging"),
            });

        let staging_transaction = WriteStagingTransaction::new(
            wgpu.staging_pool.belt(),
            wgpu.device.clone(),
            command_encoder,
        );

        Self {
            staging_transaction,
        }
    }

    // note: would be nice if we could drop the `mut` here. then systems that stage
    // data could be parallelized. but we would need to setup one staging
    // transaction per thread possibly.
    pub fn command_encoder_mut(&mut self) -> &mut wgpu::CommandEncoder {
        &mut self.staging_transaction.command_encoder
    }

    pub(super) fn flush(&mut self, wgpu: &WgpuContext) -> wgpu::CommandEncoder {
        let staging = std::mem::replace(self, Self::new(&wgpu));
        staging.staging_transaction.commit()
    }
}

impl WriteStaging for Staging {
    fn view_mut(
        &mut self,
        size: wgpu::BufferSize,
        alignment: wgpu::BufferSize,
        with_buffer_slice: impl FnOnce(&mut wgpu::CommandEncoder, wgpu::BufferSlice),
    ) -> wgpu::BufferViewMut {
        self.staging_transaction
            .view_mut(size, alignment, with_buffer_slice)
    }
}
