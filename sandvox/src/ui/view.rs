use bevy_ecs::{
    component::Component,
    query::Changed,
    relationship::RelationshipTarget,
    schedule::IntoScheduleConfigs,
    system::Populated,
};
use nalgebra::Vector2;

use crate::{
    app::WindowSize,
    ecs::{
        plugin::WorldBuilder,
        schedule,
    },
    render::{
        pass::ui_pass::UiPassUniform,
        render_target::RenderSources,
    },
    ui::UiSystems,
};

#[derive(Debug, Default, Component)]
pub struct View {
    pub size: Vector2<u32>,
    pub render: bool,
}

pub(super) fn setup_view_systems(builder: &mut WorldBuilder) {
    builder.add_systems(
        schedule::Render,
        update_views_from_windows.before(UiSystems::Layout),
    );
}

#[profiling::function]
fn update_views_from_windows(
    windows: Populated<(&WindowSize, &RenderSources), Changed<WindowSize>>,
    mut views: Populated<(&mut View, &mut UiPassUniform)>,
) {
    for (window_size, render_sources) in windows {
        for entity in render_sources.iter() {
            if let Ok((mut viewport, mut ui_pass_uniform)) = views.get_mut(entity) {
                viewport.size = window_size.size;
                ui_pass_uniform.data.viewport_size = window_size.size;
            }
        }
    }
}
