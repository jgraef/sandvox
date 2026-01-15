use std::{
    fs::File,
    io::{
        BufWriter,
        Write,
    },
    num::NonZero,
    path::Path,
};

use color_eyre::eyre::Error;
use serde::{
    Deserialize,
    Serialize,
};

use crate::{
    game::camera_controller::CameraControllerConfig,
    render::RenderConfig,
    sound::SoundConfig,
    wgpu::WgpuConfig,
};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub graphics: GraphicsConfig,

    pub sound: WithDisableFlag<SoundConfig>,

    pub num_threads: Option<NonZero<usize>>,

    #[serde(default = "default_chunk_distance")]
    pub chunk_load_distance: u32,

    #[serde(default = "default_chunk_distance")]
    pub chunk_render_distance: u32,

    #[serde(default)]
    pub camera_controller: CameraControllerConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            graphics: Default::default(),
            sound: WithDisableFlag {
                inner: Default::default(),
                disabled: false,
            },
            num_threads: None,
            chunk_load_distance: default_chunk_distance(),
            chunk_render_distance: default_chunk_distance(),
            camera_controller: Default::default(),
        }
    }
}

impl Config {
    pub fn load(path: impl AsRef<Path>) -> Result<Self, Error> {
        // todo: do the whole proper directories thingy

        let config = if !path.as_ref().exists() {
            let config = Self::default();
            config.save(&path)?;
            config
        }
        else {
            tracing::debug!(path = %path.as_ref().display(), "reading config file");

            let toml = std::fs::read(path)?;
            toml::from_slice(&toml)?
        };

        tracing::debug!(?config);

        Ok(config)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), Error> {
        tracing::debug!(path = %path.as_ref().display(), "writing config file");

        let mut writer = BufWriter::new(File::create(&path)?);
        writer.write_all(
            "# This file will be modified by the game. Any manual changes might be lost.\n\n"
                .as_bytes(),
        )?;

        writer.write_all(toml::to_string_pretty(&self)?.as_bytes())?;

        Ok(())
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphicsConfig {
    #[serde(flatten)]
    pub wgpu: WgpuConfig,

    #[serde(flatten)]
    pub render: RenderConfig,
}

fn default_chunk_distance() -> u32 {
    4
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WithDisableFlag<T> {
    #[serde(flatten)]
    pub inner: T,

    #[serde(default = "crate::util::serde::default_true")]
    pub disabled: bool,
}
