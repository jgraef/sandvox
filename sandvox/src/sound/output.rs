use std::sync::Arc;

use bevy_ecs::{
    resource::Resource,
    system::{
        Commands,
        Res,
        ResMut,
    },
};
use color_eyre::eyre::{
    Error,
    bail,
    eyre,
};
use rodio::{
    DeviceSinkBuilder,
    DeviceTrait,
    MixerDeviceSink,
    Source,
    cpal::{
        self,
        traits::HostTrait,
    },
};

use crate::sound::{
    SoundConfig,
    Volume,
    sounds::SoundSource,
};

#[derive(Clone, derive_more::Debug, Resource)]
pub struct SoundOutput {
    #[debug(skip)]
    sink: Arc<MixerDeviceSink>,
    master_volume: Volume,
}

impl SoundOutput {
    pub fn new(config: &SoundConfig) -> Result<Self, Error> {
        let host = config.host.as_ref().map_or_else(
            || Ok(cpal::default_host()),
            |host_name| open_host(host_name),
        )?;

        let device = config.device.as_ref().map_or_else(
            || {
                host.default_output_device()
                    .ok_or_else(|| eyre!("No default output device found for host"))
            },
            |name| open_device(&host, &name)?.ok_or_else(|| eyre!("Device not found: {name}")),
        )?;

        tracing::debug!(host = ?host.id(), device = device.description().unwrap().name(), master_volume = config.master_volume.0, "opened audio output device");

        let mut sink = DeviceSinkBuilder::from_device(device)?.open_stream()?;
        sink.log_on_drop(false);

        Ok(Self {
            sink: Arc::new(sink),
            master_volume: config.master_volume,
        })
    }

    pub fn add(&self, source: SoundSource) {
        let mixer = self.sink.mixer();

        match source {
            SoundSource::Buffered(buffered) => mixer.add(buffered.amplify(self.master_volume.0)),
            SoundSource::Streaming(decoder) => mixer.add(decoder.amplify(self.master_volume.0)),
        }
    }
}

/// System that configures the [`SoundOutput`]
pub fn configure_sound_output(
    config: Res<SoundConfig>,
    active_output: Option<ResMut<SoundOutput>>,
    mut commands: Commands,
) {
    // todo: check if the host/device actually changed (if we had other config
    // options), and don't recreate them if not needed.

    match SoundOutput::new(&config) {
        Ok(output) => {
            if let Some(mut active_output) = active_output {
                *active_output = output;
            }
            else {
                commands.insert_resource(output);
            }
        }
        Err(error) => {
            tracing::error!("Could not open sound output: {error}");
            if active_output.is_some() {
                commands.remove_resource::<SoundOutput>();
            }
        }
    }
}

/// System that disables sound output.
///
/// This removes any [`SoundOutput`] resource
pub fn disable_sound_output(mut commands: Commands) {
    tracing::debug!("disabling sound");
    commands.remove_resource::<SoundOutput>();
}

fn open_device(host: &cpal::Host, name: &str) -> Result<Option<cpal::Device>, Error> {
    for device in host.output_devices()? {
        match device.id() {
            Ok(id) => {
                if id.1 == name {
                    return Ok(Some(device));
                }
            }
            Err(error) => {
                tracing::error!(%error);
            }
        }

        match device.description() {
            Ok(description) => {
                if description.name() == name {
                    return Ok(Some(device));
                }
            }
            Err(error) => {
                tracing::error!(%error);
            }
        }
    }

    Ok(None)
}

fn open_host(name: &str) -> Result<cpal::Host, Error> {
    if name == "default" {
        return Ok(cpal::default_host());
    }

    for host_id in cpal::ALL_HOSTS {
        if name == host_id.name() {
            return Ok(cpal::host_from_id(*host_id)?);
        }
    }

    bail!(
        "Host not found: {name}. Available hosts: {:?}",
        cpal::ALL_HOSTS
    );
}
