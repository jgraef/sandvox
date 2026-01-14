use bevy_ecs::{
    resource::Resource,
    system::{
        Res,
        ResMut,
    },
};
use rand::Rng;

use crate::sound::{
    output::SoundOutput,
    sounds::{
        SoundId,
        Sounds,
    },
};

#[derive(Clone, Debug, Default, Resource)]
pub struct MusicPlayer {
    current: Option<SoundId>,
    // todo: this should have a list of tracks
}

pub fn play_music(mut player: ResMut<MusicPlayer>, sounds: Res<Sounds>, output: Res<SoundOutput>) {
    if player.current.is_none() {
        let tracks = sounds.music();
        let track = tracks[rand::rng().random_range(..tracks.len())];
        tracing::debug!(?track, "playing music");

        let source = sounds[track].source().unwrap();
        output.add(source);

        player.current = Some(track);
    }
}
