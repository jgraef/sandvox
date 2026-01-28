pub mod wgpu;

use bevy_ecs::resource::Resource;
use color_eyre::eyre::Error;
use serde::{
    Deserialize,
    Serialize,
};

use crate::profiler::wgpu::WgpuProfilerSink;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProfilerConfig {
    Puffin {
        address: String,
        #[serde(default)]
        open_viewer: bool,
    },
    Null,
}

impl Default for ProfilerConfig {
    fn default() -> Self {
        Self::Null
    }
}

#[derive(Debug, Resource)]
pub struct Profiler {
    inner: Inner,
}

#[derive(derive_more::Debug)]
enum Inner {
    #[cfg(feature = "puffin")]
    Puffin {
        #[debug(skip)]
        _server: puffin_http::Server,
    },
    Null,
}

impl Profiler {
    pub fn new(config: &ProfilerConfig) -> Result<Self, Error> {
        let inner = match config {
            ProfilerConfig::Puffin {
                address,
                open_viewer,
            } => {
                #[cfg(feature = "puffin")]
                {
                    use color_eyre::eyre::eyre;

                    let server = puffin_http::Server::new(address).map_err(|e| eyre!("{e}"))?;

                    tracing::info!(
                        "Profiler listening {}. Run `puffin_viewer` to view it.",
                        address
                    );

                    puffin::set_scopes_on(true);

                    if *open_viewer {
                        use std::process::Command;

                        Command::new("puffin_viewer")
                            .arg("--url")
                            .arg(address)
                            .spawn()?;
                    }

                    Inner::Puffin { _server: server }
                }

                #[cfg(not(feature = "puffin"))]
                {
                    let _ = (address, open_viewer);
                    tracing::warn!("Puffin profiler configured, but compiled out.");
                    Inner::Null
                }
            }
            ProfilerConfig::Null => Inner::Null,
        };

        Ok(Self { inner })
    }

    pub fn wgpu_sink(&self, timestamp_period: f32) -> WgpuProfilerSink {
        match &self.inner {
            #[cfg(feature = "puffin")]
            Inner::Puffin { .. } => wgpu::puffin_sink::create_sink(timestamp_period),
            Inner::Null => {
                let _ = timestamp_period;
                WgpuProfilerSink::default()
            }
        }
    }
}
