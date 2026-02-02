use std::{
    ops::Deref,
    sync::Arc,
};

use bevy_ecs::resource::Resource;
use color_eyre::eyre::Error;
use tokio::runtime::Runtime;

#[derive(Debug, Resource)]
pub struct TokioRuntime(Arc<Runtime>);

impl TokioRuntime {
    pub fn new() -> Result<Self, Error> {
        let rt = Runtime::new()?;
        Ok(Self(Arc::new(rt)))
    }
}

impl Deref for TokioRuntime {
    type Target = Runtime;

    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}
