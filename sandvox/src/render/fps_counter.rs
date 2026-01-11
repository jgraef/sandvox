use std::time::{
    Duration,
    Instant,
};

use bevy_ecs::{
    resource::Resource,
    schedule::IntoScheduleConfigs,
    system::{
        Res,
        ResMut,
    },
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
    render::RenderSystems,
};

#[derive(Clone, Copy, Debug, Default)]
pub struct FpsCounterPlugin {
    pub config: FpsCounterConfig,
}

impl Plugin for FpsCounterPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder
            .insert_resource(FpsCounter::default())
            .insert_resource(FpsCounterState::default())
            .insert_resource(self.config)
            .add_systems(
                schedule::Render,
                take_measurement.in_set(RenderSystems::EndFrame),
            );

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Resource)]
pub struct FpsCounterConfig {
    pub measurement_inverval: Duration,
}

impl Default for FpsCounterConfig {
    fn default() -> Self {
        Self {
            measurement_inverval: Duration::from_secs(1),
        }
    }
}

#[derive(Clone, Copy, Debug, Resource)]
struct FpsCounterState {
    start: Instant,
    frame_count: usize,
}

impl Default for FpsCounterState {
    fn default() -> Self {
        Self {
            start: Instant::now(),
            frame_count: 0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Resource)]
pub struct FpsCounter {
    pub fps: f32,
}

fn take_measurement(
    mut state: ResMut<FpsCounterState>,
    config: Res<FpsCounterConfig>,
    mut counter: ResMut<FpsCounter>,
) {
    state.frame_count += 1;

    let now = Instant::now();
    let elapsed = now - state.start;
    if elapsed >= config.measurement_inverval {
        counter.fps = state.frame_count as f32 / elapsed.as_secs_f32();

        state.start = now;
        state.frame_count = 0;
    }
}
