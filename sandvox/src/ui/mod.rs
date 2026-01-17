pub mod layout;
mod render;
mod text;

use bevy_ecs::{
    component::Component,
    entity::Entity,
    schedule::{
        IntoScheduleConfigs,
        SystemSet,
    },
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
        layout::setup_layout_systems,
        text::setup_text_systems,
    },
};

/*

# TODO for tomorrow:

- the text module (in render) should probably only handle fonts and define the components.
- then we need to have a system that does the leaf measure for text. it will probably need to shape the text.
- all ui elements (including text) will then have to generate meshes (maybe we can only do 1?). they'll either use the font atlas or a texture atlas for UI elements
- the ui mesh is then rendered.
- if we want to embed text in the world we need render it in the world. keep this in mind so we can easily reuse code later.

*/

#[derive(Clone, Copy, Debug, Default)]
pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        setup_layout_systems(builder);
        setup_text_systems(builder);

        builder
            .configure_system_sets(
                schedule::Render,
                UiSystems::Layout.before(UiSystems::Render),
            )
            .configure_system_sets(
                schedule::Render,
                UiSystems::Render.in_set(RenderSystems::RenderUi),
            );

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, SystemSet)]
pub enum UiSystems {
    Layout,
    Render,
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
