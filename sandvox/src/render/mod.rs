pub mod camera;
pub mod frame;
pub mod surface;

use bevy_ecs::schedule::{
    IntoScheduleConfigs,
    SystemSet,
};
use color_eyre::eyre::Error;

use crate::{
    ecs::{
        plugin::{
            Plugin,
            WorldBuilder,
        },
        schedule,
    },
    render::surface::handle_window_events,
};

#[derive(Clone, Debug, Default)]
pub struct RenderPlugin {
    // todo config
}

impl Plugin for RenderPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder.add_systems(
            schedule::Render,
            (
                handle_window_events.before(RenderSystems::BeginFrame),
                frame::begin_frame.in_set(RenderSystems::BeginFrame),
                frame::end_frame
                    .in_set(RenderSystems::EndFrame)
                    .after(RenderSystems::BeginFrame),
            ),
        );

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, SystemSet, PartialEq, Eq, Hash)]
pub enum RenderSystems {
    BeginFrame,
    EndFrame,
}
