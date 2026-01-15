pub mod layout;
mod render;

use bevy_ecs::{
    component::Component,
    entity::Entity,
};
use color_eyre::eyre::Error;
use nalgebra::Vector2;

pub use crate::ui::layout::{
    LeafMeasure,
    RoundedLayout,
};
use crate::{
    ecs::plugin::{
        Plugin,
        WorldBuilder,
    },
    ui::layout::setup_layout_systems,
};

#[derive(Clone, Copy, Debug, Default)]
pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        setup_layout_systems(builder);

        Ok(())
    }
}

// note: we're just using layout::Style now
// /// This marks something as an UI node.
// #[derive(Clone, Copy, Debug, Default, Component)]
// pub struct UiNode;

///
#[derive(Clone, Copy, Debug, Default, Component)]
pub struct UiSurface {
    pub size: Vector2<f32>,
}

/// Component to attach an UI tree to a
/// [`Surface`][crate::render::surface::Surface]
///
/// TODO: better reverse this relationship so we can have multiple UIs per
/// surface. But cameras work the same right now.
pub struct AttachedUiTree {
    pub root: Entity,
}

/* note: we might want to have our own types
pub struct Rect<T>
where
    T: Scalar,
{
    pub top_left: Point2<T>,
    pub bottom_right: Point2<T>,
}

impl<T> Rect<T>
where
    T: Scalar + ClosedSubAssign,
{
    pub fn size(&self) -> Vector2<T> {
        &self.bottom_right - &self.top_left
    }
}
*/
