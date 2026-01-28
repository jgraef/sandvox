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
use bitflags::{
    Flags,
    bitflags,
};
use color_eyre::eyre::Error;
use nalgebra::{
    Point2,
    Vector2,
};
use num_traits::identities::Zero;
use serde::{
    Deserialize,
    Serialize,
};
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

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

#[derive(Clone, Copy, Debug, thiserror::Error)]
#[error("Unsupported mouse button: {code}")]
pub struct UnsupportedMouseButton {
    pub code: u16,
}

impl TryFrom<winit::event::MouseButton> for MouseButton {
    type Error = UnsupportedMouseButton;

    #[inline]
    fn try_from(value: winit::event::MouseButton) -> Result<Self, UnsupportedMouseButton> {
        match value {
            winit::event::MouseButton::Left => Ok(MouseButton::Left),
            winit::event::MouseButton::Right => Ok(MouseButton::Right),
            winit::event::MouseButton::Middle => Ok(MouseButton::Middle),
            winit::event::MouseButton::Back => Ok(MouseButton::Back),
            winit::event::MouseButton::Forward => Ok(MouseButton::Forward),
            winit::event::MouseButton::Other(code) => Err(UnsupportedMouseButton { code }),
        }
    }
}

bitflags! {
    #[derive(Clone, Copy, Debug, Default)]
    pub struct MouseButtonSet: u8 {
        const LEFT = 0b0000_0001;
        const RIGHT = 0b0000_0010;
        const MIDDLE = 0b0000_0100;
        const BACK = 0b0001_0000;
        const FORWARD = 0b0010_0000;
    }
}

impl From<MouseButton> for MouseButtonSet {
    #[inline]
    fn from(value: MouseButton) -> Self {
        match value {
            MouseButton::Left => MouseButtonSet::LEFT,
            MouseButton::Right => MouseButtonSet::RIGHT,
            MouseButton::Middle => MouseButtonSet::MIDDLE,
            MouseButton::Back => MouseButtonSet::BACK,
            MouseButton::Forward => MouseButtonSet::FORWARD,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Component)]
pub struct MouseButtons {
    pub pressed: MouseButtonSet,
    pub just_pressed: MouseButtonSet,
    pub just_released: MouseButtonSet,
}

impl MouseButtons {
    #[inline]
    pub fn pressed(&self, mouse_button: MouseButton) -> bool {
        self.pressed.contains(mouse_button.into())
    }

    #[inline]
    pub fn just_pressed(&self, mouse_button: MouseButton) -> bool {
        self.just_pressed.contains(mouse_button.into())
    }

    #[inline]
    pub fn just_released(&self, mouse_button: MouseButton) -> bool {
        self.just_released.contains(mouse_button.into())
    }
}

#[derive(SystemParam)]
struct UpdateMouse<'w, 's> {
    mouse_cursors: Query<'w, 's, (&'static mut MousePosition, &'static mut MouseButtons)>,
    commands: Commands<'w, 's>,
    new_mouse_cursors: Local<'s, EntityHashMap<(MousePosition, MouseButtons)>>,
}

impl<'w, 's> UpdateMouse<'w, 's> {
    fn begin(&mut self) {
        assert!(self.new_mouse_cursors.is_empty());

        for (mut mouse_position, mut mouse_buttons) in &mut self.mouse_cursors {
            if !mouse_position.frame_delta.is_zero() {
                mouse_position.frame_delta = Vector2::zeros();
            }

            // clear just_pressed and just_released.
            // the extra check is so that we only trigger change detection if the sets
            // really change.
            if !mouse_buttons.just_pressed.is_empty() {
                mouse_buttons.just_pressed.clear();
            }
            if !mouse_buttons.just_released.is_empty() {
                mouse_buttons.just_released.clear();
            }
        }
    }

    #[inline]
    fn update_position(&mut self, window: Entity, update: impl FnOnce(&mut MousePosition)) {
        self.update_position_if(window, |_| true, update);
    }

    fn update_position_if(
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

        if let Ok((mut mouse_position, _)) = self.mouse_cursors.get_mut(window) {
            update_if(&mut mouse_position);
        }
        else if let Some((mouse_position, _)) = self.new_mouse_cursors.get_mut(&window) {
            update_if(mouse_position);
        }
        else {
            let mut mouse_position = MousePosition::default();
            update_if(&mut mouse_position);
            self.new_mouse_cursors
                .insert(window, (mouse_position, MouseButtons::default()));
        }
    }

    fn update_buttons_if(
        &mut self,
        window: Entity,
        condition: impl FnOnce(&MouseButtons) -> bool,
        update: impl FnOnce(&mut MouseButtons),
    ) {
        let update_if = |mouse_buttons: &mut MouseButtons| {
            if condition(mouse_buttons) {
                update(mouse_buttons);
            }
        };

        if let Ok((_, mut mouse_buttons)) = self.mouse_cursors.get_mut(window) {
            update_if(&mut mouse_buttons);
        }
        else if let Some((_, mouse_buttons)) = self.new_mouse_cursors.get_mut(&window) {
            update_if(mouse_buttons)
        }
        else {
            let mut mouse_buttons = MouseButtons::default();
            update_if(&mut mouse_buttons);
            self.new_mouse_cursors
                .insert(window, (MousePosition::default(), mouse_buttons));
        }
    }

    #[inline]
    fn insert(&mut self, window: Entity) {
        if self.mouse_cursors.get(window).is_err() {
            self.new_mouse_cursors.insert(window, Default::default());
        }
    }

    #[inline]
    fn remove(&mut self, window: Entity) {
        if self.new_mouse_cursors.remove(&window).is_none() {
            self.commands
                .entity(window)
                .try_remove::<MousePosition>()
                .try_remove::<MouseButtons>();
        }
    }

    fn end(mut self) {
        for (window, (mouse_position, mouse_buttons)) in self.new_mouse_cursors.drain() {
            self.commands
                .entity(window)
                .insert((mouse_position, mouse_buttons));
        }
    }
}

fn update_mouse(mut window_events: MessageReader<WindowEvent>, mut update_mouse: UpdateMouse) {
    update_mouse.begin();

    for event in window_events.read() {
        match event {
            WindowEvent::MousePosition { window, position } => {
                update_mouse.update_position_if(
                    *window,
                    |mouse_position| mouse_position.position != *position,
                    |mouse_position| {
                        mouse_position.position = *position;
                    },
                );
            }
            WindowEvent::MouseDelta { window, delta } => {
                if !delta.is_zero() {
                    update_mouse.update_position(*window, |mouse_position| {
                        mouse_position.frame_delta += *delta;
                    });
                }
            }
            WindowEvent::MouseWheel {
                window: _,
                delta: _,
            } => {
                // todo
            }
            WindowEvent::MouseButtonPressed { window, button } => {
                let button = MouseButtonSet::from(*button);
                update_mouse.update_buttons_if(
                    *window,
                    |mouse_buttons| !mouse_buttons.pressed.contains(button),
                    |mouse_buttons| {
                        mouse_buttons.pressed.insert(button);
                        mouse_buttons.just_pressed.insert(button)
                    },
                );
            }
            WindowEvent::MouseButtonReleased { window, button } => {
                let button = MouseButtonSet::from(*button);
                update_mouse.update_buttons_if(
                    *window,
                    |mouse_buttons| mouse_buttons.pressed.contains(button),
                    |mouse_buttons| {
                        mouse_buttons.pressed.remove(button);
                        mouse_buttons.just_released.insert(button)
                    },
                );
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
