mod layout;
mod render;
mod sprites;
mod text;

use bevy_ecs::{
    component::Component,
    entity::Entity,
    hierarchy::ChildOf,
    name::NameOrEntity,
    query::{
        AnyOf,
        Changed,
        QueryData,
        With,
        Without,
    },
    relationship::RelationshipTarget,
    schedule::{
        IntoScheduleConfigs,
        SystemSet,
    },
    system::{
        Commands,
        Populated,
    },
};
use color_eyre::eyre::Error;
use nalgebra::Vector2;

pub use crate::ui::{
    layout::{
        LayoutCache,
        LeafMeasure,
        RoundedLayout,
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
};
use crate::{
    app::WindowSize,
    ecs::{
        plugin::{
            Plugin,
            WorldBuilder,
        },
        schedule,
    },
    render::{
        RenderSystems,
        surface::{
            RenderSources,
            RenderTarget,
        },
    },
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
    },
};

#[derive(Clone, Copy, Debug, Default)]
pub struct UiPlugin;

impl Plugin for UiPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
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
            .add_systems(
                schedule::Render,
                (
                    create_viewports_from_render_targets,
                    update_viewport_from_surfaces,
                )
                    .before(UiSystems::Layout),
            )
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

#[derive(Clone, Copy, Debug, Default, Component)]
pub struct Viewport {
    pub size: Vector2<u32>,
}

fn create_viewports_from_render_targets(
    windows: Populated<(NameOrEntity, &WindowSize)>,
    roots: Populated<
        (NameOrEntity, &RenderTarget),
        (With<Style>, Without<ChildOf>, Without<Viewport>),
    >,
    mut commands: Commands,
) {
    for (viewport_entity, render_target) in roots {
        if let Ok((window_name, window_size)) = windows.get(render_target.0) {
            tracing::debug!(window = %window_name, viewport = %viewport_entity, size = ?window_size.size, "create ui viewport");

            commands.entity(viewport_entity.entity).insert(Viewport {
                size: window_size.size,
            });
        }
    }
}

fn update_viewport_from_surfaces(
    windows: Populated<(&WindowSize, &RenderSources), Changed<WindowSize>>,
    mut viewports: Populated<&mut Viewport>,
) {
    for (window_size, render_sources) in windows {
        for entity in render_sources.iter() {
            if let Ok(mut viewport) = viewports.get_mut(entity) {
                viewport.size = window_size.size;
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Component)]
pub struct RedrawRequested;

#[derive(Clone, Copy, Debug, Component, PartialEq, Eq)]
pub struct Root {
    pub viewport: Entity,
    pub render_target: Option<Entity>,
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
