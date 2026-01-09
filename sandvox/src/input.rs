use std::collections::HashSet;

use bevy_ecs::{
    entity::Entity,
    message::MessageReader,
    resource::Resource,
    schedule::{
        IntoScheduleConfigs,
        SystemSet,
    },
    system::{
        Commands,
        ResMut,
    },
};
use color_eyre::eyre::Error;
use nalgebra::{
    Point2,
    Vector2,
};
use num_traits::identities::Zero;
use winit::keyboard::{
    KeyCode,
    PhysicalKey,
};

use crate::{
    app::WindowEvent,
    ecs::{
        plugin::{
            Plugin,
            WorldBuilder,
        },
        schedule,
    },
};

#[derive(Clone, Copy, Debug, Default)]
pub struct InputPlugin;

impl Plugin for InputPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder.insert_resource(Keys::default()).add_systems(
            schedule::PreUpdate,
            (update_mouse, update_keys).in_set(InputSystems::Update),
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, SystemSet)]
pub enum InputSystems {
    Update,
}

#[derive(Clone, Copy, Debug, Resource)]
pub struct MousePosition {
    pub window: Entity,
    pub position: Point2<f32>,
    pub frame_delta: Vector2<f32>,
}

fn update_mouse(
    mut window_events: MessageReader<WindowEvent>,
    mouse_position: Option<ResMut<MousePosition>>,
    mut commands: Commands,
) {
    let mut mouse_position_changed = false;
    let mut updated_mouse_position =
        mouse_position
            .as_deref()
            .cloned()
            .map(|mut mouse_position| {
                if !mouse_position.frame_delta.is_zero() {
                    mouse_position_changed = true;
                }
                mouse_position.frame_delta = Zero::zero();
                mouse_position
            });

    for event in window_events.read() {
        match event {
            WindowEvent::MouseEntered { window } => {
                tracing::trace!(?window, "mouse entered");
                // can't set it because we don't know the position
            }
            WindowEvent::MouseLeft { window } => {
                tracing::trace!(?window, "mouse left");
                updated_mouse_position = None;
            }
            WindowEvent::MouseMoved { window, position } => {
                tracing::trace!(?window, ?position, "mouse moved");

                updated_mouse_position.get_or_insert_with(|| {
                    MousePosition {
                        window: *window,
                        position: *position,
                        frame_delta: Default::default(),
                    }
                });

                mouse_position_changed = true;
            }
            WindowEvent::MouseMovedDelta { window, delta } => {
                let updated_mouse_position = updated_mouse_position.get_or_insert_with(|| {
                    MousePosition {
                        window: *window,
                        position: Default::default(),
                        frame_delta: Default::default(),
                    }
                });
                updated_mouse_position.frame_delta += delta;

                mouse_position_changed = true;
            }
            _ => {}
        }
    }

    if let Some(updated_mouse_position) = updated_mouse_position {
        if mouse_position_changed {
            if let Some(mut mouse_position) = mouse_position {
                *mouse_position = updated_mouse_position;
            }
            else {
                commands.insert_resource(updated_mouse_position);
            }
        }
    }
    else {
        if mouse_position.is_some() {
            commands.remove_resource::<MousePosition>();
        }
    }
}

#[derive(Clone, Debug, Default, Resource)]
pub struct Keys {
    pub pressed: HashSet<KeyCode>,
    pub just_pressed: HashSet<KeyCode>,
    pub just_released: HashSet<KeyCode>,
}

fn update_keys(mut window_events: MessageReader<WindowEvent>, mut keys: ResMut<Keys>) {
    // clear just_pressed and just_released.
    // the extra check is so that we only trigger change detection if the sets
    // really change.
    if !keys.just_pressed.is_empty() {
        keys.just_pressed.clear();
    }
    if !keys.just_released.is_empty() {
        keys.just_released.clear();
    }

    for event in window_events.read() {
        match event {
            WindowEvent::LostFocus { window: _ } => {
                // release all keys
                let keys = &mut *keys;
                keys.just_released.extend(keys.pressed.drain());
            }
            WindowEvent::KeyPressed { window: _, key } => {
                match key {
                    PhysicalKey::Code(key_code) => {
                        if keys.pressed.insert(*key_code) {
                            keys.just_pressed.insert(*key_code);
                        }
                    }
                    PhysicalKey::Unidentified(_native_key_code) => {}
                }
            }
            WindowEvent::KeyReleased { window: _, key } => {
                match key {
                    PhysicalKey::Code(key_code) => {
                        if keys.pressed.remove(key_code) {
                            keys.just_released.insert(*key_code);
                        }
                    }
                    PhysicalKey::Unidentified(_native_key_code) => {}
                }
            }
            _ => {}
        }
    }
}
