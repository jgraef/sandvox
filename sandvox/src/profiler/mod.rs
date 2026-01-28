pub mod wgpu;

use bevy_ecs::resource::Resource;
use color_eyre::eyre::Error;
use serde::{
    Deserialize,
    Serialize,
};

use crate::profiler::wgpu::WgpuProfilerSink;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProfilerConfig {
    // note: this is for puffin only right now
    pub server: String,
}

impl Default for ProfilerConfig {
    fn default() -> Self {
        #[cfg(feature = "puffin")]
        {
            Self {
                server: format!("localhost:{}", puffin_http::DEFAULT_PORT),
            }
        }

        #[cfg(not(feature = "puffin"))]
        {
            Self {
                server: "localhost:8585".to_owned(),
            }
        }
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
        #[cfg(feature = "puffin")]
        {
            use color_eyre::eyre::eyre;

            let server = puffin_http::Server::new(&config.server).map_err(|e| eyre!("{e}"))?;

            tracing::info!(
                "Profiler listening {}. Run `puffin_viewer` to view it.",
                config.server
            );

            puffin::set_scopes_on(true);

            return Ok(Self {
                inner: Inner::Puffin { _server: server },
            });
        }

        #[allow(unreachable_code)]
        {
            let _ = config;
            tracing::warn!("Profiler configured, but compiled out.");
            Ok(Self { inner: Inner::Null })
        }
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
