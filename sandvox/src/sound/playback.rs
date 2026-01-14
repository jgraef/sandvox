use bevy_ecs::{
    component::Component,
    entity::Entity,
    query::Without,
    system::{
        Commands,
        Populated,
        Res,
    },
};

use crate::sound::{
    SoundOutput,
    sounds::{
        SoundId,
        Sounds,
    },
};

/// Component that plays a sound
#[derive(Clone, Copy, Debug, Component)]
pub struct PlaySound {
    pub sound: SoundId,
}

#[derive(Clone, Copy, Debug, Component)]
pub struct PlaybackState {
    // todo: volume, but we need to share it the playback thread.
    // this volume can then be set to the final volume of the sound - including any silencing from
    // the sound being far away.
}

/// System that starts sound playback for any [`PlaySound`] components that are
/// not playing yet.
pub fn start_sound_playback(
    output: Res<SoundOutput>,
    play_sound: Populated<(Entity, &PlaySound), Without<PlaybackState>>,
    sounds: Res<Sounds>,
    mut commands: Commands,
) {
    for (entity, play_sound) in play_sound {
        // todo: don't just crash if the sound can't be loaded. instead we should ignore
        // it, but we also need to remove it from Sounds
        tracing::debug!(?play_sound, "playing sound");
        let source = sounds[play_sound.sound].source().unwrap();
        output.add(source);

        commands.entity(entity).insert(PlaybackState {});
    }
}
