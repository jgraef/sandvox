mod layout;
mod render;
mod widgets;

use bevy_ecs::{
    component::Component,
    schedule::IntoScheduleConfigs,
};
use color_eyre::eyre::Error;
use nalgebra::Vector2;

pub use crate::ui::layout::{
    LeafMeasure,
    RoundedLayout,
};
use crate::{
    ecs::{
        plugin::{
            Plugin,
            WorldBuilder,
        },
        schedule,
    },
    render::RenderSystems,
    ui::{
        layout::layout_trees,
        widgets::text::Fonts,
    },
};

#[derive(Clone, Copy, Debug, Default)]
pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        let fonts = Fonts::new();

        builder.insert_resource(fonts).add_systems(
            schedule::Render,
            layout_trees.in_set(RenderSystems::BeginFrame),
        );

        Ok(())
    }
}

/// This marks something as an UI root node.
///
/// Do we need this? UI root nodes have a UI surface
#[derive(Clone, Copy, Debug, Default, Component)]
struct UiRoot;

#[derive(Clone, Copy, Debug, Default, Component)]
struct UiSurface {
    size: Vector2<f32>,
}
