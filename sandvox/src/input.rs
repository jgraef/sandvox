use std::collections::HashSet;

use bevy_ecs::{
    component::Component,
    entity::{
        Entity,
        EntityHashMap,
    },
    message::MessageReader,
    query::{
        With,
        Without,
    },
    schedule::{
        IntoScheduleConfigs,
        SystemSet,
    },
    system::{
        Commands,
        Local,
        Query,
        SystemParam,
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
    app::{
        WindowEvent,
        WindowHandle,
    },
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
        builder.add_systems(
            schedule::PreUpdate,
            (update_mouse, (create_keys, update_keys).chain()).in_set(InputSystems::Update),
        );
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, SystemSet)]
pub enum InputSystems {
    Update,
}

#[derive(Clone, Copy, Debug, Default, Component)]
pub struct MousePosition {
    pub position: Point2<f32>,
    pub frame_delta: Vector2<f32>,
}

#[derive(SystemParam)]
struct UpdateMouse<'w, 's> {
    mouse_positions: Query<'w, 's, &'static mut MousePosition>,
    commands: Commands<'w, 's>,
    new_mouse_positions: Local<'s, EntityHashMap<MousePosition>>,
}

impl<'w, 's> UpdateMouse<'w, 's> {
    fn begin(&mut self) {
        assert!(self.new_mouse_positions.is_empty());

        for mut mouse_position in &mut self.mouse_positions {
            if !mouse_position.frame_delta.is_zero() {
                mouse_position.frame_delta = Vector2::zeros();
            }
        }
    }

    fn update(&mut self, window: Entity, update: impl FnOnce(&mut MousePosition)) {
        self.update_if(window, |_| true, update);
    }

    fn update_if(
        &mut self,
        window: Entity,
        condition: impl FnOnce(&MousePosition) -> bool,
        update: impl FnOnce(&mut MousePosition),
    ) {
        let update_if = |mouse_position: &mut MousePosition| {
            if condition(mouse_position) {
                update(mouse_position);
            }
        };

        if let Ok(mut mouse_position) = self.mouse_positions.get_mut(window) {
            update_if(&mut mouse_position);
        }
        else if let Some(mouse_position) = self.new_mouse_positions.get_mut(&window) {
            update_if(mouse_position);
        }
        else {
            let mut mouse_position = MousePosition::default();
            update_if(&mut mouse_position);
            self.new_mouse_positions.insert(window, mouse_position);
        }
    }

    fn insert(&mut self, window: Entity) {
        if self.mouse_positions.get(window).is_err() {
            self.new_mouse_positions.insert(window, Default::default());
        }
    }

    fn remove(&mut self, window: Entity) {
        if self.new_mouse_positions.remove(&window).is_none() {
            self.commands.entity(window).try_remove::<MousePosition>();
        }
    }

    fn end(mut self) {
        for (window, mouse_position) in self.new_mouse_positions.drain() {
            self.commands.entity(window).insert(mouse_position);
        }
    }
}

fn update_mouse(mut window_events: MessageReader<WindowEvent>, mut update_mouse: UpdateMouse) {
    update_mouse.begin();

    for event in window_events.read() {
        match event {
            WindowEvent::MousePosition { window, position } => {
                update_mouse.update_if(
                    *window,
                    |mouse_position| mouse_position.position != *position,
                    |mouse_position| {
                        mouse_position.position = *position;
                    },
                );
            }
            WindowEvent::MouseDelta { window, delta } => {
                if !delta.is_zero() {
                    update_mouse.update(*window, |mouse_position| {
                        mouse_position.frame_delta += *delta;
                    });
                }
            }
            WindowEvent::MouseEntered { window } => {
                update_mouse.insert(*window);
            }
            WindowEvent::MouseLeft { window } => {
                update_mouse.remove(*window);
            }
            _ => {}
        }
    }

    update_mouse.end();
}

#[derive(Clone, Debug, Default, Component)]
pub struct Keys {
    pub pressed: HashSet<KeyCode>,
    pub just_pressed: HashSet<KeyCode>,
    pub just_released: HashSet<KeyCode>,
}

#[derive(SystemParam)]
struct UpdateKeys<'w, 's> {
    keys: Query<'w, 's, &'static mut Keys>,
}

impl<'w, 's> UpdateKeys<'w, 's> {
    fn begin(&mut self) {
        for mut keys in &mut self.keys {
            // clear just_pressed and just_released.
            // the extra check is so that we only trigger change detection if the sets
            // really change.
            if !keys.just_pressed.is_empty() {
                keys.just_pressed.clear();
            }
            if !keys.just_released.is_empty() {
                keys.just_released.clear();
            }
        }
    }

    fn update_if(
        &mut self,
        window: Entity,
        condition: impl FnOnce(&Keys) -> bool,
        update: impl FnOnce(&mut Keys),
    ) {
        if let Ok(mut keys) = self.keys.get_mut(window) {
            if condition(&*keys) {
                update(&mut keys);
            }
        }
        else {
            tracing::error!(?window, "keys for unknown window");
        }
    }
}

fn create_keys(
    windows_without_keys: Query<Entity, (With<WindowHandle>, Without<Keys>)>,
    mut commands: Commands,
) {
    for entity in windows_without_keys {
        commands.entity(entity).insert(Keys::default());
    }
}

fn update_keys(mut window_events: MessageReader<WindowEvent>, mut update_keys: UpdateKeys) {
    update_keys.begin();

    for event in window_events.read() {
        match event {
            WindowEvent::LostFocus { window } => {
                // release all keys
                update_keys.update_if(
                    *window,
                    |keys| !keys.pressed.is_empty(),
                    |keys| {
                        keys.just_released.extend(keys.pressed.drain());
                    },
                );
            }
            WindowEvent::KeyPressed { window, key } => {
                match key {
                    PhysicalKey::Code(key_code) => {
                        update_keys.update_if(
                            *window,
                            |keys| !keys.pressed.contains(&key_code),
                            |keys| {
                                keys.pressed.insert(*key_code);
                                keys.just_pressed.insert(*key_code);
                            },
                        );
                    }
                    PhysicalKey::Unidentified(_native_key_code) => {}
                }
            }
            WindowEvent::KeyReleased { window, key } => {
                match key {
                    PhysicalKey::Code(key_code) => {
                        update_keys.update_if(
                            *window,
                            |keys| keys.pressed.contains(&key_code),
                            |keys| {
                                keys.pressed.remove(key_code);
                                keys.just_released.insert(*key_code);
                            },
                        );
                    }
                    PhysicalKey::Unidentified(_native_key_code) => {}
                }
            }
            _ => {}
        }
    }
}
