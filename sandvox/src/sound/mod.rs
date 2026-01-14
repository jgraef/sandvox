pub mod music;
pub mod output;
pub mod playback;
pub mod sounds;

use bevy_ecs::{
    resource::Resource,
    schedule::{
        IntoScheduleConfigs,
        common_conditions::{
            resource_changed,
            resource_exists,
            resource_removed,
        },
    },
};
use color_eyre::eyre::Error;
use serde::{
    Deserialize,
    Serialize,
};

use crate::{
    ecs::{
        plugin::{
            Plugin,
            WorldBuilder,
        },
        schedule,
    },
    sound::{
        music::{
            MusicPlayer,
            play_music,
        },
        output::{
            SoundOutput,
            configure_sound_output,
            disable_sound_output,
        },
        playback::start_sound_playback,
        sounds::load_sounds,
    },
};

#[derive(Clone, Debug, Default)]
pub struct SoundPlugin {
    pub config: SoundConfig,
}

impl Plugin for SoundPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder
            .insert_resource(self.config.clone())
            .insert_resource(MusicPlayer::default())
            .add_systems(
                schedule::PostStartup,
                (
                    // fixme: this will configure the sound output twice - once here but also in
                    // update. we need some way to tell the update run condition
                    // that it should ignore the initial changedness
                    // configure_sound_output,
                    load_sounds
                )
                    .run_if(resource_exists::<SoundConfig>),
            )
            .add_systems(
                schedule::Update,
                (
                    configure_sound_output.run_if(resource_changed::<SoundConfig>),
                    // don't run the first time
                    //.and(not(run_once))),
                    (
                        disable_sound_output.run_if(resource_removed::<SoundConfig>),
                        start_sound_playback,
                        play_music,
                    )
                        .run_if(resource_exists::<SoundOutput>),
                ),
            );

        Ok(())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, Resource)]
#[serde(deny_unknown_fields)]
pub struct SoundConfig {
    pub host: Option<String>,
    pub device: Option<String>,

    #[serde(default)]
    pub master_volume: Volume,

    #[serde(default)]
    pub effect_volume: Volume,

    #[serde(default)]
    pub music_volume: Volume,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Volume(pub f32);

impl Default for Volume {
    fn default() -> Self {
        Self(1.0)
    }
}
