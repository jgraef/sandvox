mod layout;
mod render;
mod sprites;
mod text;
mod view;

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

pub use crate::ui::{
    layout::{
        FinalLayout,
        LayoutCache,
        LeafMeasure,
        Style,
    },
    render::{
        QuadBuilder,
        RenderBufferBuilder,
        ShowDebugOutlines,
    },
    sprites::{
        Background,
        Sprites,
    },
    view::View,
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
        render::setup_render_systems,
        sprites::setup_sprite_systems,
        text::{
            TextLeafMeasure,
            setup_text_systems,
        },
        view::setup_view_systems,
    },
};

#[derive(Clone, Copy, Debug, Default)]
pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        setup_view_systems(builder);
        setup_layout_systems(
            builder,
            LayoutConfig {
                leaf_measure: DefaultLeafMeasure::default(),
            },
        );
        setup_render_systems(builder);
        setup_text_systems(builder);
        setup_sprite_systems(builder);

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

/// Attached to UI nodes and points to root node
#[derive(Clone, Copy, Debug, Component, PartialEq, Eq)]
pub struct Root {
    pub root: Entity,
}

#[derive(Clone, Copy, Debug, Default)]
struct DefaultLeafMeasure {
    text: TextLeafMeasure,
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
