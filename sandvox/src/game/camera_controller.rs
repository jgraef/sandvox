use std::f32::consts::{
    FRAC_PI_2,
    TAU,
};

use bevy_ecs::{
    bundle::Bundle,
    change_detection::DetectChanges,
    component::Component,
    entity::Entity,
    lifecycle::HookContext,
    message::{
        Message,
        MessageReader,
    },
    query::With,
    schedule::{
        IntoScheduleConfigs,
        common_conditions::on_message,
    },
    system::{
        Commands,
        Populated,
        Res,
    },
    world::DeferredWorld,
};
use color_eyre::eyre::Error;
use indexmap::IndexMap;
use nalgebra::{
    Translation3,
    UnitQuaternion,
    Vector3,
};
use num_traits::identities::Zero;
use serde::{
    Deserialize,
    Serialize,
};
use winit::keyboard::KeyCode;

use crate::{
    app::{
        GrabCursor,
        Time,
    },
    ecs::{
        plugin::{
            Plugin,
            WorldBuilder,
        },
        schedule,
        transform::LocalTransform,
    },
    input::{
        InputSystems,
        Keys,
        MousePosition,
    },
    render::surface::RenderTarget,
};

#[derive(Clone, Copy, Debug, Default)]
pub struct CameraControllerPlugin;

impl Plugin for CameraControllerPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder.add_message::<ControllerMessage>().add_systems(
            schedule::Update,
            (
                grab_cursor.run_if(on_message::<ControllerMessage>),
                update_camera,
            )
                .after(InputSystems::Update),
        );

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Component)]
#[component(on_add = controller_added, on_remove = controller_removed)]
pub struct CameraControllerState {
    pub yaw: f32,
    pub pitch: f32,
}

impl CameraControllerState {
    pub fn apply(&self, transform: &mut LocalTransform) {
        let yaw_quaternion = UnitQuaternion::from_axis_angle(&Vector3::y_axis(), self.yaw);
        let pitch_quaternion = UnitQuaternion::from_axis_angle(&Vector3::x_axis(), -self.pitch);

        transform.isometry.rotation = yaw_quaternion * pitch_quaternion;
    }
}

#[derive(Clone, Debug, Component, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraControllerConfig {
    // rad / pixel
    pub mouse_sensitivity: f32,

    pub keybindings: IndexMap<KeyCode, Movement>,

    // block / second
    pub movement_speed: f32,
}

impl Default for CameraControllerConfig {
    fn default() -> Self {
        let mut keybindings = IndexMap::with_capacity(6);
        keybindings.insert(KeyCode::KeyW, Movement::Local(Vector3::z()));
        keybindings.insert(KeyCode::KeyA, Movement::Local(-Vector3::x()));
        keybindings.insert(KeyCode::KeyS, Movement::Local(-Vector3::z()));
        keybindings.insert(KeyCode::KeyD, Movement::Local(Vector3::x()));
        keybindings.insert(KeyCode::ShiftLeft, Movement::Global(-Vector3::y()));
        keybindings.insert(KeyCode::Space, Movement::Global(Vector3::y()));

        Self {
            mouse_sensitivity: 0.01,
            keybindings,
            movement_speed: 16.0,
        }
    }
}

#[derive(Clone, Debug, Default, Bundle)]
pub struct CameraController {
    pub state: CameraControllerState,
    pub config: CameraControllerConfig,
}

fn controller_added(mut world: DeferredWorld, context: HookContext) {
    world.write_message(ControllerMessage::ControllerAdded(context.entity));
}

fn controller_removed(mut world: DeferredWorld, context: HookContext) {
    world.write_message(ControllerMessage::ControllerRemoved(context.entity));
}

#[derive(Clone, Copy, Debug, Message)]
enum ControllerMessage {
    ControllerAdded(Entity),
    ControllerRemoved(Entity),
}

fn grab_cursor(
    mut messages: MessageReader<ControllerMessage>,
    cameras: Populated<&RenderTarget, With<CameraControllerState>>,
    mut commands: Commands,
) {
    for message in messages.read() {
        tracing::debug!(?message);

        match message {
            ControllerMessage::ControllerAdded(entity) => {
                if let Ok(render_target) = cameras.get(*entity) {
                    commands.entity(render_target.0).insert(GrabCursor);
                }
            }
            ControllerMessage::ControllerRemoved(entity) => {
                if let Ok(render_target) = cameras.get(*entity) {
                    commands.entity(render_target.0).try_remove::<GrabCursor>();
                }
            }
        }
    }
}

fn update_camera(
    time: Res<Time>,
    windows: Populated<(Option<&MousePosition>, &Keys)>,
    cameras: Populated<(
        &mut LocalTransform,
        &mut CameraControllerState,
        &CameraControllerConfig,
        &RenderTarget,
    )>,
) {
    for (mut transform, mut state, config, render_target) in cameras {
        if state.is_added() {
            state.apply(&mut transform);
        }

        if let Ok((mouse_position, keys)) = windows.get(render_target.0) {
            let dt = time.delta_seconds();

            // mouse
            if let Some(mouse_position) = mouse_position {
                if !mouse_position.frame_delta.is_zero() {
                    // note: don't multiply by delta-time, since the mouse delta is already
                    // naturally scaled by that.
                    let delta = config.mouse_sensitivity * mouse_position.frame_delta;

                    tracing::trace!(?delta, ?mouse_position.frame_delta, "mouse movement");

                    state.yaw = (state.yaw + delta.x).rem_euclid(TAU);
                    state.pitch = (state.pitch - delta.y).clamp(-FRAC_PI_2, FRAC_PI_2);

                    state.apply(&mut transform);
                }
            }

            // keyboard
            if !keys.pressed.is_empty() {
                tracing::trace!(?keys.pressed, "keys pressed");
                let speed = dt * config.movement_speed;
                for (key_code, action) in &config.keybindings {
                    if keys.pressed.contains(key_code) {
                        action.apply(&mut transform, speed);
                    }
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "lowercase")]
pub enum Movement {
    Local(Vector3<f32>),
    Global(Vector3<f32>),
}

impl Movement {
    fn apply(&self, transform: &mut LocalTransform, speed: f32) {
        match self {
            Movement::Local(direction) => {
                transform.translate_local(&Translation3::from(speed * direction));
            }
            Movement::Global(direction) => {
                transform.translate_global(&Translation3::from(speed * direction));
            }
        }
    }
}
