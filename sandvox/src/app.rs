use std::{
    collections::HashMap,
    sync::Arc,
};

use bevy_ecs::{
    component::Component,
    entity::Entity,
    lifecycle::HookContext,
    message::{
        Message,
        MessageWriter,
    },
    query::{
        With,
        Without,
    },
    resource::Resource,
    system::{
        Commands,
        In,
        InRef,
        Query,
        Res,
        ResMut,
        Single,
        SystemParam,
    },
    world::{
        DeferredWorld,
        World,
    },
};
use color_eyre::eyre::Error;
use nalgebra::{
    Point2,
    Point3,
    Vector2,
    Vector3,
};
use winit::{
    application::ApplicationHandler,
    event::StartCause,
    event_loop::{
        ActiveEventLoop,
        ControlFlow,
        EventLoop,
    },
    keyboard::PhysicalKey,
    window::{
        CursorGrabMode,
        WindowAttributes,
    },
};

use crate::{
    ecs::{
        plugin::{
            Plugin,
            WorldBuilder,
        },
        schedule,
        transform::{
            LocalTransform,
            TransformHierarchyPlugin,
        },
    },
    input::InputPlugin,
    render::{
        RenderPlugin,
        camera::{
            CameraPlugin,
            CameraProjection,
        },
        camera_controller::{
            CameraController,
            CameraControllerPlugin,
        },
        surface::{
            AttachedCamera,
            ClearColor,
        },
    },
    voxel::flat::{
        CHUNK_SIDE_LENGTH,
        FlatChunk,
        FlatChunkPlugin,
        IsOpaque,
    },
    wgpu::WgpuPlugin,
};

#[derive(Debug)]
pub struct App {
    world: World,
}

impl App {
    pub fn new() -> Result<Self, Error> {
        let world = WorldBuilder::default()
            .add_plugin(AppPlugin::default())?
            .add_plugin(TransformHierarchyPlugin)?
            .add_plugin(InputPlugin)?
            .add_plugin(WgpuPlugin::default())?
            .add_plugin(RenderPlugin::default())?
            .add_plugin(CameraPlugin)?
            .add_plugin(CameraControllerPlugin)?
            .add_plugin(FlatChunkPlugin::<TestVoxel>::default())?
            .add_systems(schedule::PostStartup, init_world)
            .build();

        Ok(Self { world })
    }

    pub fn run(mut self) -> Result<(), Error> {
        let event_loop = EventLoop::with_user_event().build()?;

        let proxy = event_loop.create_proxy();
        self.world.insert_resource(EventLoopProxy(proxy));

        event_loop.run_app(&mut self)?;
        Ok(())
    }

    fn update(&mut self) {
        self.world.run_schedule(schedule::PreUpdate);
        self.world.run_schedule(schedule::Update);
        self.world.run_schedule(schedule::PostUpdate);

        self.world.run_schedule(schedule::Render);
    }
}

impl ApplicationHandler<AppEvent> for App {
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
        window_id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        self.world
            .run_system_cached_with(handle_window_event, (event_loop, window_id, event))
            .unwrap();
    }

    fn device_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        device_id: winit::event::DeviceId,
        event: winit::event::DeviceEvent,
    ) {
        self.world
            .run_system_cached_with(handle_device_event, (event_loop, device_id, event))
            .unwrap();
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Poll);
    }

    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        let _ = event_loop;

        match cause {
            StartCause::Poll => {
                self.update();
            }
            _ => {}
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppEvent) {
        let _ = event_loop;

        match event {
            AppEvent::GrabCursor { window } => {
                self.world
                    .run_system_cached_with(
                        |In(window), windows: Query<&WindowHandle>| {
                            if let Ok(window) = windows.get(window) {
                                tracing::debug!("grabbing cursor");

                                // todo: make this more platform-independent

                                window
                                    .window
                                    .set_cursor_grab(CursorGrabMode::Locked)
                                    .unwrap();

                                window.window.set_cursor_visible(false);

                                // this panics even though we just locked the
                                // cursor (wayland)
                                /*
                                let window_size = window.window.inner_size();
                                window
                                    .window
                                    .set_cursor_position(PhysicalPosition {
                                        x: i32::try_from(window_size.width).unwrap() / 2,
                                        y: i32::try_from(window_size.height).unwrap() / 2,
                                    })
                                    .unwrap();
                                */
                            }
                        },
                        window,
                    )
                    .unwrap();
            }
            AppEvent::UngrabCursor { window } => {
                self.world
                    .run_system_cached_with(
                        |In(window), windows: Query<&WindowHandle>| {
                            if let Ok(window) = windows.get(window) {
                                tracing::debug!("ungrabbing cursor");

                                window.window.set_cursor_grab(CursorGrabMode::None).unwrap();
                                window.window.set_cursor_visible(true);
                            }
                        },
                        window,
                    )
                    .unwrap();
            }
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
            .add_message::<WindowEvent>()
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

fn handle_window_event(
    (InRef(event_loop), In(window_id), In(event)): (
        InRef<ActiveEventLoop>,
        In<winit::window::WindowId>,
        In<winit::event::WindowEvent>,
    ),
    mut state: ResMut<AppState>,
    window_id_map: Res<WindowIdMap>,
    mut window_events: MessageWriter<WindowEvent>,
    mut window_sizes: Query<&mut WindowSize>,
    mut commands: Commands,
) {
    if let Some(window_entity) = window_id_map.id_map.get(&window_id) {
        match event {
            winit::event::WindowEvent::Resized(physical_size) => {
                let size = Vector2::new(physical_size.width, physical_size.height);

                if let Ok(mut window_size) = window_sizes.get_mut(*window_entity) {
                    window_size.size = size;
                }
                else {
                    commands.entity(*window_entity).insert(WindowSize { size });
                }

                window_events.write(WindowEvent::Resized {
                    window: *window_entity,
                    size,
                });
            }
            winit::event::WindowEvent::CloseRequested => {
                tracing::debug!("close requested");
                *state = AppState::Exiting;
                event_loop.exit();
            }
            winit::event::WindowEvent::Destroyed => {
                // todo: instead just tell rendering system to destroy that surface
                tracing::debug!("window destroyed");
                *state = AppState::Exiting;
                event_loop.exit();
            }
            winit::event::WindowEvent::KeyboardInput {
                device_id: _,
                event,
                is_synthetic,
            } => {
                if !is_synthetic {
                    match event.state {
                        winit::event::ElementState::Pressed => {
                            window_events.write(WindowEvent::KeyPressed {
                                window: *window_entity,
                                key: event.physical_key,
                            });
                        }
                        winit::event::ElementState::Released => {
                            window_events.write(WindowEvent::KeyReleased {
                                window: *window_entity,
                                key: event.physical_key,
                            });
                        }
                    }
                }

                // todo
            }
            winit::event::WindowEvent::ModifiersChanged(_modifiers) => {
                // todo
            }
            winit::event::WindowEvent::CursorMoved {
                device_id: _,
                position,
            } => {
                window_events.write(WindowEvent::MouseMoved {
                    window: *window_entity,
                    position: Point2::new(position.x, position.y).cast(),
                });
            }
            winit::event::WindowEvent::CursorEntered { device_id: _ } => {
                window_events.write(WindowEvent::MouseEntered {
                    window: *window_entity,
                });
            }
            winit::event::WindowEvent::CursorLeft { device_id: _ } => {
                window_events.write(WindowEvent::MouseLeft {
                    window: *window_entity,
                });
            }
            winit::event::WindowEvent::MouseWheel {
                device_id: _,
                delta: _,
                phase: _,
            } => {
                // todo
            }
            winit::event::WindowEvent::MouseInput {
                device_id: _,
                state: _,
                button: _,
            } => todo!(),
            winit::event::WindowEvent::ScaleFactorChanged {
                scale_factor: _,
                inner_size_writer: _,
            } => {
                // todo
            }
            winit::event::WindowEvent::ThemeChanged(_theme) => {
                // todo
            }
            winit::event::WindowEvent::RedrawRequested => {
                // todo
            }
            winit::event::WindowEvent::Focused(focused) => {
                if focused {
                    tracing::debug!(window = ?window_entity, "window gained focus");

                    window_events.write(WindowEvent::GainedFocus {
                        window: *window_entity,
                    });
                    commands.entity(*window_entity).insert(Focused);
                }
                else {
                    tracing::debug!(window = ?window_entity, "window lost focus");

                    window_events.write(WindowEvent::LostFocus {
                        window: *window_entity,
                    });
                    commands.entity(*window_entity).try_remove::<Focused>();
                }
            }
            _ => {}
        }
    }
}

fn handle_device_event(
    (InRef(event_loop), In(device_id), In(event)): (
        InRef<ActiveEventLoop>,
        In<winit::event::DeviceId>,
        In<winit::event::DeviceEvent>,
    ),
    focused_window: Option<Single<Entity, With<Focused>>>,
    mut window_events: MessageWriter<WindowEvent>,
) {
    use winit::event::DeviceEvent::*;

    let _ = (event_loop, device_id);

    match event {
        MouseMotion { delta } => {
            if let Some(focused_window) = focused_window {
                window_events.write(WindowEvent::MouseMovedDelta {
                    window: *focused_window,
                    delta: Vector2::new(delta.0 as f32, delta.1 as f32),
                });
            }
        }
        _ => {}
    }
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

#[derive(Clone, Copy, Debug, Default, Component)]
pub struct Focused;

#[derive(Debug, Default, Resource)]
struct WindowIdMap {
    id_map: HashMap<winit::window::WindowId, Entity>,
}

#[derive(Clone, Debug, Message)]
pub enum WindowEvent {
    Created {
        window: Entity,
    },
    Resized {
        window: Entity,
        size: Vector2<u32>,
    },
    MouseMoved {
        window: Entity,
        position: Point2<f32>,
    },
    MouseMovedDelta {
        window: Entity,
        delta: Vector2<f32>,
    },
    MouseEntered {
        window: Entity,
    },
    MouseLeft {
        window: Entity,
    },
    GainedFocus {
        window: Entity,
    },
    LostFocus {
        window: Entity,
    },
    KeyPressed {
        window: Entity,
        key: PhysicalKey,
    },
    KeyReleased {
        window: Entity,
        key: PhysicalKey,
    },
}

#[derive(Clone, Copy, Debug)]
enum AppEvent {
    GrabCursor { window: Entity },
    UngrabCursor { window: Entity },
}

#[derive(Clone, Debug, Resource)]
struct EventLoopProxy(winit::event_loop::EventLoopProxy<AppEvent>);

#[derive(Clone, Copy, Debug, Default, Component)]
#[component(on_add = grab_cursor, on_remove = ungrab_cursor)]
pub struct GrabCursor;

fn grab_cursor(world: DeferredWorld, context: HookContext) {
    let proxy = world.resource::<EventLoopProxy>();
    proxy
        .0
        .send_event(AppEvent::GrabCursor {
            window: context.entity,
        })
        .unwrap();
}

fn ungrab_cursor(world: DeferredWorld, context: HookContext) {
    let proxy = world.resource::<EventLoopProxy>();
    proxy
        .0
        .send_event(AppEvent::UngrabCursor {
            window: context.entity,
        })
        .unwrap();
}

fn init_world(mut commands: Commands) {
    let chunk_side_length = CHUNK_SIDE_LENGTH as f32;
    let chunk_center = Point3::from(Vector3::repeat(0.5 * chunk_side_length));

    commands.spawn((
        {
            /*let noise = Perlin::new(1312);
            let scaling = 1.0 / chunk_side_length;

            FlatChunk::from_fn(move |point| {
                let value = noise.get((point.cast::<f32>() * scaling).cast::<f64>().into());

                if value > 0.0 {
                    TestVoxel::Dirt
                }
                else {
                    TestVoxel::Air
                }
            })*/
            FlatChunk::from_fn(|_point| TestVoxel::Dirt)
        },
        LocalTransform::from(Point3::origin()),
    ));

    let camera_entity = commands
        .spawn((
            CameraProjection::default(),
            LocalTransform::look_at(
                &(chunk_center - chunk_side_length * Vector3::z()),
                &chunk_center,
                &Vector3::y(),
            ),
            CameraController::default(),
        ))
        .id();

    commands.spawn((
        Window {
            title: "SandVox".to_owned(),
        },
        ClearColor::default(),
        AttachedCamera(camera_entity),
    ));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TestVoxel {
    Air,
    Dirt,
}

impl IsOpaque for TestVoxel {
    fn is_opaque(&self) -> bool {
        matches!(self, Self::Dirt)
    }
}
