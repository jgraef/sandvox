pub mod layout;
mod render;
mod text;

use bevy_ecs::{
    component::Component,
    entity::Entity,
    query::{
        AnyOf,
        QueryData,
    },
    schedule::{
        IntoScheduleConfigs,
        SystemSet,
    },
};
use color_eyre::eyre::Error;
use nalgebra::Vector2;

pub use crate::ui::layout::{
    LayoutCache,
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
        layout::{
            LayoutConfig,
            setup_layout_systems,
        },
        text::{
            TextLeafMeasure,
            setup_text_systems,
        },
    },
};

#[derive(Clone, Copy, Debug, Default)]
pub struct UiPlugin<L = DefaultLeafMeasure> {
    pub leaf_measure: L,
}

impl<L> Plugin for UiPlugin<L>
where
    L: LeafMeasure + Clone,
{
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        setup_layout_systems(
            builder,
            LayoutConfig {
                leaf_measure: self.leaf_measure.clone(),
            },
        );
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

pub type DefaultUiPlugin = UiPlugin<DefaultLeafMeasure>;

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

#[derive(Clone, Copy, Debug, Default)]
pub struct DefaultLeafMeasure {
    pub text: TextLeafMeasure,
}

impl LeafMeasure for DefaultLeafMeasure {
    type Data = <TextLeafMeasure as LeafMeasure>::Data;
    type Node = AnyOf<(<TextLeafMeasure as LeafMeasure>::Node,)>;

    fn measure(
        &self,
        leaf: &mut <Self::Node as QueryData>::Item<'_, '_>,
        data: &mut <Self::Data as bevy_ecs::system::SystemParam>::Item<'_, '_>,
        known_dimensions: taffy::Size<Option<f32>>,
        available_space: taffy::Size<taffy::AvailableSpace>,
    ) -> taffy::Size<f32> {
        let (text,) = leaf;
        if let Some(text) = text {
            self.text
                .measure(text, data, known_dimensions, available_space)
        }
        else {
            unreachable!()
        }
    }
}
