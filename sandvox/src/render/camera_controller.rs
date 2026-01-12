use std::collections::HashMap;

use bevy_ecs::{
    bundle::Bundle,
    component::Component,
    entity::Entity,
    lifecycle::HookContext,
    message::{
        Message,
        MessageReader,
    },
    query::With,
    schedule::IntoScheduleConfigs,
    system::{
        Commands,
        Populated,
        Query,
        Res,
        Single,
    },
    world::DeferredWorld,
};
use color_eyre::eyre::Error;
use nalgebra::{
    Translation3,
    UnitQuaternion,
    Vector3,
};
use num_traits::identities::Zero;
use winit::keyboard::KeyCode;

use crate::{
    app::{
        DeltaTime,
        Focused,
        GrabCursor,
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
    render::surface::AttachedCamera,
};

#[derive(Clone, Copy, Debug, Default)]
pub struct CameraControllerPlugin;

impl Plugin for CameraControllerPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder.add_message::<ControllerMessage>().add_systems(
            schedule::Update,
            (grab_cursor, update_camera).after(InputSystems::Update),
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

#[derive(Clone, Debug, Component)]
pub struct CameraControllerConfig {
    // rad / pixel
    pub mouse_sensitivity: f32,

    pub keybindings: HashMap<KeyCode, Movement>,

    // block / second
    pub movement_speed: f32,
}

impl Default for CameraControllerConfig {
    fn default() -> Self {
        let mut keybindings = HashMap::with_capacity(6);
        keybindings.insert(KeyCode::KeyW, Movement::Local(Vector3::z()));
        keybindings.insert(KeyCode::KeyA, Movement::Local(-Vector3::x()));
        keybindings.insert(KeyCode::KeyS, Movement::Local(-Vector3::z()));
        keybindings.insert(KeyCode::KeyD, Movement::Local(Vector3::x()));
        keybindings.insert(KeyCode::ShiftLeft, Movement::Global(-Vector3::y()));
        keybindings.insert(KeyCode::Space, Movement::Global(Vector3::y()));

        Self {
            mouse_sensitivity: 0.01,
            keybindings,
            movement_speed: 8.0,
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
    windows: Populated<(Entity, &AttachedCamera)>,
    mut commands: Commands,
) {
    for message in messages.read() {
        let find_window = |camera| {
            windows.iter().find_map(|(window_entity, attached_camera)| {
                (attached_camera.0 == camera).then_some(window_entity)
            })
        };

        tracing::debug!(?message);
        match message {
            ControllerMessage::ControllerAdded(entity) => {
                if let Some(window) = find_window(*entity) {
                    commands.entity(window).insert(GrabCursor);
                }
            }
            ControllerMessage::ControllerRemoved(entity) => {
                if let Some(window) = find_window(*entity) {
                    commands.entity(window).try_remove::<GrabCursor>();
                }
            }
        }
    }
}

fn update_camera(
    mouse_position: Option<Res<MousePosition>>,
    keys: Res<Keys>,
    delta_time: Res<DeltaTime>,
    camera: Single<&AttachedCamera, With<Focused>>,
    mut cameras: Query<(
        &mut LocalTransform,
        &mut CameraControllerState,
        &CameraControllerConfig,
    )>,
) {
    if let Ok((mut transform, mut state, config)) = cameras.get_mut(camera.0) {
        let dt = delta_time.seconds();

        // mouse
        if let Some(mouse_position) = mouse_position {
            if !mouse_position.frame_delta.is_zero() {
                // note: don't multiply by delta-time, since the mouse delta is already
                // naturally scaled by that.
                let delta = config.mouse_sensitivity * mouse_position.frame_delta;

                tracing::trace!(?delta, ?mouse_position.frame_delta, "mouse movement");

                state.yaw += delta.x;
                state.pitch += delta.y;

                let yaw_quaternion = UnitQuaternion::from_axis_angle(&Vector3::y_axis(), state.yaw);
                let pitch_quaternion =
                    UnitQuaternion::from_axis_angle(&Vector3::x_axis(), state.pitch);

                transform.isometry.rotation = yaw_quaternion * pitch_quaternion;
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

#[derive(Clone, Copy, Debug)]
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
