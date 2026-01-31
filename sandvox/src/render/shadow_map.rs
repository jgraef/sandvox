use color_eyre::eyre::Error;

use crate::ecs::plugin::{
    Plugin,
    WorldBuilder,
};

#[derive(Clone, Copy, Debug, Default)]
pub struct ShadowMapPlugin;

impl Plugin for ShadowMapPlugin {
    fn setup(&self, _builder: &mut WorldBuilder) -> Result<(), Error> {
        todo!()
    }
}
