use std::{
    collections::HashMap,
    fs::File,
    ops::Index,
    path::{
        Path,
        PathBuf,
    },
};

use bevy_ecs::{
    resource::Resource,
    system::Commands,
};
use color_eyre::{
    Section,
    eyre::{
        Error,
        bail,
    },
};
use rodio::{
    Decoder,
    Source,
    source::Buffered,
};

use crate::sound::sounds::config::SoundDef;

#[derive(Clone, Debug, Resource)]
pub struct Sounds {
    sounds: Vec<Sound>,
    by_name: HashMap<String, SoundId>,
    music: Vec<SoundId>,
}

impl Sounds {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, Error> {
        let toml_directory = path.as_ref().parent().unwrap();
        let toml = std::fs::read(&path)?;
        let sound_defs: config::SoundDefs = toml::from_slice(&toml)?;

        let total_sounds = sound_defs.effects.len() + sound_defs.music.tracks.len();

        let mut sounds = Vec::with_capacity(total_sounds);
        let mut by_name = HashMap::with_capacity(total_sounds);
        let mut music = Vec::with_capacity(sound_defs.music.tracks.len());

        let mut load_sound_def = |name, sound_def: SoundDef| {
            let full_path = toml_directory.join(&sound_def.path);
            let sound_id = SoundId(sounds.len());

            let mut sound = Sound {
                path: full_path,
                buffered: None,
            };

            if sound_def.preload {
                sound.preload()?;
                tracing::debug!(path = ?sound.path, ?sound_id, "loaded sound");
            }
            else if !sound.path.exists() {
                bail!("Sound file not found: {:?}", sound.path);
            }

            sounds.push(sound);
            by_name.insert(name, sound_id);
            Ok(sound_id)
        };

        for (name, sound_def) in sound_defs.effects {
            load_sound_def(name, sound_def)?;
        }

        for (name, sound_def) in sound_defs.music.tracks {
            let sound_id = load_sound_def(name, sound_def)?;
            music.push(sound_id);
        }

        Ok(Self {
            sounds,
            by_name,
            music,
        })
    }

    pub fn lookup(&self, name: &str) -> Option<SoundId> {
        self.by_name.get(name).copied()
    }

    pub fn music(&self) -> &[SoundId] {
        &self.music
    }
}

impl Index<SoundId> for Sounds {
    type Output = Sound;

    fn index(&self, index: SoundId) -> &Self::Output {
        &self.sounds[index.0]
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SoundId(usize);

#[derive(Clone, derive_more::Debug)]
pub struct Sound {
    path: PathBuf,
    #[debug(skip)]
    buffered: Option<Buffered<Decoder<File>>>,
}

impl Sound {
    pub fn preload(&mut self) -> Result<(), Error> {
        self.buffered = Some(self.decoder()?.buffered());
        Ok(())
    }

    pub fn decoder(&self) -> Result<Decoder<File>, Error> {
        tracing::debug!(path = ?self.path, "reading sound file");
        let file = File::open(&self.path).with_note(|| self.path.display().to_string())?;
        let decoder = Decoder::new_vorbis(file).with_note(|| self.path.display().to_string())?;
        tracing::debug!(
            channels = decoder.channels(),
            sample_rate = decoder.sample_rate(),
            total_duration = ?decoder.total_duration(),
            current_span_length = ?decoder.current_span_len(),
        );

        Ok(decoder)
    }

    pub fn source(&self) -> Result<SoundSource, Error> {
        let source = if let Some(buffered) = &self.buffered {
            SoundSource::Buffered(buffered.clone())
        }
        else {
            SoundSource::Streaming(self.decoder()?)
        };

        Ok(source)
    }
}

#[derive(derive_more::Debug)]
pub enum SoundSource {
    Buffered(#[debug(skip)] Buffered<Decoder<File>>),
    Streaming(#[debug(skip)] Decoder<File>),
}

pub fn load_sounds(mut commands: Commands) {
    // todo: hardcoded path
    let sounds = Sounds::load("assets/sounds.toml").unwrap();
    commands.insert_resource(sounds);
}

mod config {
    use std::path::PathBuf;

    use indexmap::IndexMap;
    use serde::{
        Deserialize,
        Serialize,
    };

    #[derive(Clone, Debug, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct SoundDefs {
        #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
        pub effects: IndexMap<String, SoundDef>,

        #[serde(default)]
        pub music: MusicDefs,
    }

    #[derive(Clone, Debug, Default, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct MusicDefs {
        #[serde(skip_serializing_if = "Option::is_none")]
        pub pause_between: Option<RangeOrSingle<f32>>,

        #[serde(default)]
        pub fade_in: f32,

        #[serde(default)]
        pub fade_out: f32,

        #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
        pub tracks: IndexMap<String, SoundDef>,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct SoundDef {
        pub path: PathBuf,

        #[serde(default)]
        pub preload: bool,
    }

    #[derive(Clone, Copy, Debug, Serialize, Deserialize)]
    #[serde(untagged)]
    pub enum RangeOrSingle<T> {
        Single(T),
        Range { min: T, max: T },
    }
}
