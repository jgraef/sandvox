use color_eyre::eyre::Error;

use crate::ecs::plugin::{
    Plugin,
    WorldBuilder,
};

#[derive(Clone, Copy, Debug, Default)]
pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        Ok(())
    }
}
