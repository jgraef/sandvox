use std::{
    collections::HashMap,
    sync::Arc,
};

use bevy_ecs::{
    component::Component,
    entity::Entity,
    message::{
        Message,
        MessageWriter,
    },
    query::Without,
    resource::Resource,
    system::{
        Commands,
        In,
        InRef,
        ParamSet,
        Query,
        Res,
        ResMut,
        SystemParam,
    },
    world::World,
};
use clap::Parser;
use color_eyre::eyre::Error;
use nalgebra::Vector2;
use palette::WithAlpha;
use winit::{
    application::ApplicationHandler,
    event_loop::{
        ActiveEventLoop,
        EventLoop,
    },
    window::{
        WindowAttributes,
        WindowId,
    },
};

use crate::{
    ecs::{
        plugin::{
            Plugin,
            WorldBuilder,
        },
        schedule,
    },
    render::{
        RenderPlugin,
        camera::CameraProjection,
        surface::{
            AttachedCamera,
            ClearColor,
        },
    },
    wgpu::WgpuPlugin,
};

#[derive(Debug, Parser)]
pub struct Args {
    // todo
}

#[derive(Debug)]
pub struct App {
    world: World,
}

impl App {
    pub fn new(args: Args) -> Result<Self, Error> {
        let _ = args;

        let mut builder = WorldBuilder::default();

        builder.register_plugin(AppPlugin::default())?;
        builder.register_plugin(WgpuPlugin::default())?;
        builder.register_plugin(RenderPlugin::default())?;

        builder.add_systems(schedule::PostStartup, init_world);

        let world = builder.build();

        Ok(Self { world })
    }

    pub fn run(mut self) -> Result<(), Error> {
        let event_loop = EventLoop::new()?;
        event_loop.run_app(&mut self)?;
        Ok(())
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.world
            .run_system_cached_with(resume_app, event_loop)
            .unwrap();
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        self.world
            .run_system_cached_with(suspend_app, event_loop)
            .unwrap();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: winit::event::WindowEvent,
    ) {
        let redraw = self
            .world
            .run_system_cached_with(handle_event, (event_loop, window_id, event))
            .unwrap();

        if redraw {
            self.world.run_schedule(schedule::Render);
        }
    }
}

#[derive(Debug, Resource)]
enum AppState {
    Active,
    Suspended,
    Exiting,
}

#[derive(Debug, Default)]
struct AppPlugin;

impl Plugin for AppPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder
            .register_message::<WindowEvent>()
            .insert_resource(AppState::Suspended)
            .insert_resource(WindowIdMap::default());
        Ok(())
    }
}

fn resume_app(
    InRef(event_loop): InRef<ActiveEventLoop>,
    mut state: ResMut<AppState>,
    mut create_windows: CreateWindows,
) {
    match *state {
        AppState::Suspended => {
            *state = AppState::Active;
            create_windows.create_windows(event_loop);
        }
        _ => {}
    }
}

fn suspend_app(InRef(_event_loop): InRef<ActiveEventLoop>, mut state: ResMut<AppState>) {
    match *state {
        AppState::Active => {
            *state = AppState::Suspended;
        }
        _ => {}
    }
}

fn handle_event(
    (InRef(event_loop), In(window_id), In(event)): (
        InRef<ActiveEventLoop>,
        In<WindowId>,
        In<winit::event::WindowEvent>,
    ),
    mut params: ParamSet<(CreateWindows, HandleEvents)>,
) -> bool {
    params.p0().create_windows(event_loop);
    params.p1().handle_event(event_loop, window_id, event)
}

#[derive(SystemParam)]
struct CreateWindows<'w, 's> {
    requests: Query<'w, 's, (Entity, &'static Window), Without<WindowHandle>>,
    window_id_map: ResMut<'w, WindowIdMap>,
    commands: Commands<'w, 's>,
    window_events: MessageWriter<'w, WindowEvent>,
}

impl<'world, 'state> CreateWindows<'world, 'state> {
    pub fn create_windows(&mut self, event_loop: &ActiveEventLoop) {
        for (entity, request) in self.requests {
            let window = event_loop
                .create_window(WindowAttributes::default().with_title(request.title.clone()))
                .unwrap();
            let size = window.inner_size();
            let size = Vector2::new(size.width, size.height);

            tracing::debug!(title = request.title, ?size, "created window");

            self.window_id_map.id_map.insert(window.id(), entity);

            self.commands.entity(entity).insert((
                WindowHandle {
                    window: Arc::new(window),
                },
                WindowSize { size },
            ));

            self.window_events
                .write(WindowEvent::Created { window: entity });
        }
    }
}

#[derive(SystemParam)]
struct HandleEvents<'w, 's> {
    state: ResMut<'w, AppState>,
    window_id_map: Res<'w, WindowIdMap>,
    window_events: MessageWriter<'w, WindowEvent>,
    window_sizes: Query<'w, 's, &'static mut WindowSize>,
    commands: Commands<'w, 's>,
}

impl<'w, 's> HandleEvents<'w, 's> {
    fn handle_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: winit::event::WindowEvent,
    ) -> bool {
        use winit::event::WindowEvent::*;

        if let Some(window_entity) = self.window_id_map.id_map.get(&window_id) {
            match event {
                Resized(physical_size) => {
                    let size = Vector2::new(physical_size.width, physical_size.height);

                    if let Ok(mut window_size) = self.window_sizes.get_mut(*window_entity) {
                        window_size.size = size;
                    }
                    else {
                        self.commands
                            .entity(*window_entity)
                            .insert(WindowSize { size });
                    }

                    self.window_events.write(WindowEvent::Resized {
                        window: *window_entity,
                        size,
                    });
                }
                CloseRequested => {
                    tracing::debug!("close requested");
                    *self.state = AppState::Exiting;
                    event_loop.exit();
                }
                Destroyed => {
                    // todo: instead just tell rendering system to destroy that surface
                    tracing::debug!("window destroyed");
                    *self.state = AppState::Exiting;
                    event_loop.exit();
                }
                KeyboardInput {
                    device_id: _,
                    event: _,
                    is_synthetic: _,
                } => {
                    // todo
                }
                ModifiersChanged(_modifiers) => {
                    // todo
                }
                CursorMoved {
                    device_id: _,
                    position: _,
                } => {
                    // todo
                }
                CursorEntered { device_id: _ } => {
                    // todo
                }
                CursorLeft { device_id: _ } => {
                    // todo
                }
                MouseWheel {
                    device_id: _,
                    delta: _,
                    phase: _,
                } => {
                    // todo
                }
                MouseInput {
                    device_id: _,
                    state: _,
                    button: _,
                } => todo!(),
                ScaleFactorChanged {
                    scale_factor: _,
                    inner_size_writer: _,
                } => {
                    // todo
                }
                ThemeChanged(_theme) => {
                    // todo
                }
                RedrawRequested => {
                    return true;
                }
                _ => {}
            }
        }

        false
    }
}

#[derive(Clone, Debug, Component)]
pub struct Window {
    pub title: String,
}

#[derive(Clone, Debug, Component)]
pub struct WindowHandle {
    pub window: Arc<winit::window::Window>,
}

#[derive(Clone, Copy, Debug, Component)]
pub struct WindowSize {
    pub size: Vector2<u32>,
}

#[derive(Debug, Default, Resource)]
struct WindowIdMap {
    id_map: HashMap<WindowId, Entity>,
}

#[derive(Clone, Debug, Message)]
pub enum WindowEvent {
    Created { window: Entity },
    Resized { window: Entity, size: Vector2<u32> },
}

fn init_world(mut commands: Commands) {
    let camera_entity = commands.spawn((CameraProjection::default(),)).id();

    commands.spawn((
        Window {
            title: "SandVox".to_owned(),
        },
        ClearColor(palette::named::PURPLE.into_format().with_alpha(1.0)),
        AttachedCamera(camera_entity),
    ));
}
