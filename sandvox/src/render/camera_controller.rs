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

#[derive(Clone, Copy, Debug, Component)]
pub struct CameraControllerConfig {
    // rad / (pixel * second)
    pub mouse_sensitivity: f32,

    pub keybindings: CameraControllerKeybindings,

    // block / second
    pub movement_speed: f32,
}

impl Default for CameraControllerConfig {
    fn default() -> Self {
        Self {
            mouse_sensitivity: 0.3,
            keybindings: Default::default(),
            movement_speed: 8.0,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct CameraControllerKeybindings {
    left: KeyCode,
    right: KeyCode,
    down: KeyCode,
    up: KeyCode,
    backward: KeyCode,
    forward: KeyCode,
}

impl Default for CameraControllerKeybindings {
    fn default() -> Self {
        Self {
            left: KeyCode::KeyA,
            right: KeyCode::KeyD,
            down: KeyCode::ShiftLeft,
            up: KeyCode::Space,
            backward: KeyCode::KeyS,
            forward: KeyCode::KeyW,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Bundle)]
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
                let delta = dt * config.mouse_sensitivity * mouse_position.frame_delta;
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

        tracing::trace!(?keys.pressed, "keys pressed");
        let mut check_key = |key_code, direction| {
            if keys.pressed.contains(&key_code) {
                transform
                    .translate_local(&Translation3::from(dt * config.movement_speed * direction));
            }
        };

        check_key(config.keybindings.left, -Vector3::x());
        check_key(config.keybindings.right, Vector3::x());
        check_key(config.keybindings.down, -Vector3::y());
        check_key(config.keybindings.up, Vector3::y());
        check_key(config.keybindings.backward, -Vector3::z());
        check_key(config.keybindings.forward, Vector3::z());
    }
}
