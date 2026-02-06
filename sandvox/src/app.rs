use std::{
    collections::HashMap,
    num::NonZero,
    path::PathBuf,
    sync::Arc,
    time::{
        Duration,
        Instant,
    },
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
        Changed,
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
use chrono::{
    DateTime,
    Utc,
};
use clap::Parser;
use color_eyre::{
    Section,
    eyre::{
        Error,
        bail,
    },
};
use nalgebra::{
    Point2,
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
    build_info::BUILD_INFO,
    config::Config,
    ecs::{
        background_tasks::BackgroundTaskPlugin,
        plugin::{
            Plugin,
            WorldBuilder,
        },
        schedule,
        transform::TransformHierarchyPlugin,
    },
    game::{
        GamePlugin,
        InitWorld,
        terrain::{
            WorldBounds,
            WorldConfig,
            WorldSeed,
        },
    },
    input::{
        InputPlugin,
        MouseButton,
    },
    profiler::Profiler,
    render::{
        RenderPlugin,
        camera::CameraPlugin,
        fps_counter::FpsCounterPlugin,
        mesh::MeshPlugin,
    },
    sound::SoundPlugin,
    ui::UiPlugin,
    wgpu::WgpuPlugin,
};

#[derive(Clone, Debug, Default, Parser)]
pub struct Args {
    #[clap(short = 'G', long)]
    pub generate_schedule_graphs: Option<PathBuf>,

    #[clap(long)]
    pub num_threads: Option<NonZero<usize>>,

    #[clap(short = 'w', long = "world-file")]
    pub world_file: Option<PathBuf>,

    #[clap(short = 'c', long = "create-world")]
    pub create_world: Option<PathBuf>,
}

#[derive(Debug)]
pub struct App {
    world: World,
}

impl App {
    pub fn new(args: Args) -> Result<Self, Error> {
        tracing::info!(?BUILD_INFO);

        // todo: load from proper location
        let config = Config::load("config.toml")?;

        let profiler = config
            .profiler
            .as_ref()
            .map(|config| Profiler::new(config))
            .transpose()?;

        let mut world_builder = WorldBuilder::default();

        if let Some(profiler) = profiler {
            world_builder.insert_resource(profiler);
        }

        #[cfg(feature = "tokio")]
        {
            use crate::util::tokio::TokioRuntime;

            world_builder.insert_resource(TokioRuntime::new()?);
        }

        #[cfg(feature = "rcon")]
        {
            use crate::rcon::RconPlugin;

            if let Some(config) = config.rcon {
                world_builder.add_plugin(RconPlugin { config })?;
            }
        }

        world_builder
            .add_plugin(BackgroundTaskPlugin {
                num_threads: args.num_threads.or(config.num_threads),
            })?
            .insert_resource({
                let now = Instant::now();
                let now_utc = Utc::now();
                Time {
                    app_start: now,
                    app_start_utc: now_utc,
                    tick_start: now,
                    tick_delta: Duration::ZERO,
                    tick_count: 0,
                }
            })
            .add_plugin(AppPlugin)?
            .add_plugin(TransformHierarchyPlugin)?
            .add_plugin(InputPlugin)?
            .add_plugin(WgpuPlugin {
                config: config.graphics.wgpu,
            })?
            .add_plugin(RenderPlugin {
                config: config.graphics.render,
            })?
            .add_plugin(FpsCounterPlugin::default())?
            .add_plugin(MeshPlugin)?
            .add_plugin(CameraPlugin)?
            .add_plugin(UiPlugin)?;

        if let Some(config) = config.sound {
            world_builder.add_plugin(SoundPlugin { config })?;
        }

        let init_world = if let Some(world_config_file) = &args.create_world {
            if let Some(world_file) = &args.world_file
                && world_file.exists()
            {
                bail!("--create-world passed, but world-file already exists");
            }

            let world_config_toml = std::fs::read(world_config_file)
                .with_note(|| world_config_file.display().to_string())?;
            let world_config: WorldConfig = toml::from_slice(&world_config_toml)?;
            InitWorld::Create {
                world_config,
                world_file: args.world_file,
            }
        }
        else {
            if let Some(world_file) = args.world_file {
                InitWorld::Load { world_file }
            }
            else {
                tracing::info!(
                    "Neither --world-file, nor --create-world passed. Creating default world."
                );
                InitWorld::Create {
                    world_config: {
                        // special world config for development
                        WorldConfig {
                            seed: WorldSeed::FIXED_DEFAULT,
                            bounds: WorldBounds {
                                min: Vector3::new(None, Some(-2), Some(-1)),
                                max: Default::default(),
                            },
                        }
                    },
                    world_file: None,
                }
            }
        };

        world_builder
            .add_plugin({
                GamePlugin {
                    game_config: config.game,
                    init_world,
                }
            })?
            .add_systems(schedule::PostUpdate, update_window_config);

        if let Some(path) = args.generate_schedule_graphs {
            world_builder.write_schedule_graphs_to_dot(path)?;
        }

        let world = world_builder.build();

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
        {
            profiling::function_scope!();

            let tick_start = Instant::now();

            {
                let mut time = self.world.resource_mut::<Time>();
                time.tick_start = tick_start;
            }

            self.world.run_schedule(schedule::PreUpdate);
            self.world.run_schedule(schedule::Update);
            self.world.run_schedule(schedule::PostUpdate);

            self.world.run_schedule(schedule::Render);

            {
                let mut time = self.world.resource_mut::<Time>();
                time.tick_delta = tick_start.elapsed();
                time.tick_count += 1;
            }
        }

        profiling::finish_frame!();
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

#[derive(Debug, Resource, PartialEq, Eq)]
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
    mut windows: Query<(&WindowHandle, Option<&mut WindowSize>)>,
    mut commands: Commands,
) {
    let window_entity = window_id_map
        .id_map
        .get(&window_id)
        .unwrap_or_else(|| panic!("Window not in map: {window_id:?}"));

    match event {
        winit::event::WindowEvent::Resized(physical_size) => {
            let (window, mut window_size) = windows.get_mut(*window_entity).unwrap();

            let new_size = Vector2::new(physical_size.width, physical_size.height);

            if let Some(window_size) = &mut window_size {
                window_size.size = new_size;
            }
            else {
                commands
                    .entity(*window_entity)
                    .insert(WindowSize { size: new_size });
            }

            window_events.write(WindowEvent::Resized {
                window: *window_entity,
                size: new_size,
            });

            window.window.request_redraw();
        }
        winit::event::WindowEvent::CloseRequested => {
            tracing::debug!("close requested");
            *state = AppState::Exiting;
        }
        winit::event::WindowEvent::Destroyed => {
            // todo: instead just tell rendering system to destroy that surface
            tracing::debug!("window destroyed");
            *state = AppState::Exiting;
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
            window_events.write(WindowEvent::MousePosition {
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
            delta,
            phase: _,
        } => {
            match delta {
                winit::event::MouseScrollDelta::LineDelta(x, y) => {
                    window_events.write(WindowEvent::MouseWheel {
                        window: *window_entity,
                        delta: Vector2::new(x, y),
                    });
                }
                winit::event::MouseScrollDelta::PixelDelta(_physical_position) => {
                    // todo: scale relative to line scroll. this should probably
                    // be handled by the input system
                }
            }
        }
        winit::event::WindowEvent::MouseInput {
            device_id: _,
            state,
            button,
        } => {
            if let Ok(button) = MouseButton::try_from(button) {
                match state {
                    winit::event::ElementState::Pressed => {
                        window_events.write(WindowEvent::MouseButtonPressed {
                            window: *window_entity,
                            button,
                        });
                    }
                    winit::event::ElementState::Released => {
                        window_events.write(WindowEvent::MouseButtonReleased {
                            window: *window_entity,
                            button,
                        });
                    }
                }
            }
        }
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

    if *state == AppState::Exiting {
        event_loop.exit();
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
                window_events.write(WindowEvent::MouseDelta {
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
    requests: Query<'w, 's, (Entity, &'static WindowConfig), Without<WindowHandle>>,
    window_id_map: ResMut<'w, WindowIdMap>,
    commands: Commands<'w, 's>,
    window_events: MessageWriter<'w, WindowEvent>,
}

impl<'world, 'state> CreateWindows<'world, 'state> {
    pub fn create_windows(&mut self, event_loop: &ActiveEventLoop) {
        for (entity, config) in self.requests {
            let window = event_loop
                .create_window(WindowAttributes::default().with_title(config.title.clone()))
                .unwrap();
            let size = window.inner_size();
            let size = Vector2::new(size.width, size.height);

            tracing::debug!(title = config.title, ?size, "created window");

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

fn update_window_config(windows: Query<(&WindowConfig, &WindowHandle), Changed<WindowConfig>>) {
    for (config, handle) in windows {
        handle.window.set_title(&config.title);
    }
}

#[derive(Clone, Debug, Component)]
pub struct WindowConfig {
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
    MousePosition {
        window: Entity,
        position: Point2<f32>,
    },
    MouseDelta {
        window: Entity,
        delta: Vector2<f32>,
    },
    MouseEntered {
        window: Entity,
    },
    MouseLeft {
        window: Entity,
    },
    MouseWheel {
        window: Entity,
        delta: Vector2<f32>,
    },
    MouseButtonPressed {
        window: Entity,
        button: MouseButton,
    },
    MouseButtonReleased {
        window: Entity,
        button: MouseButton,
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

#[derive(Clone, Copy, Debug, Resource)]
pub struct Time {
    pub app_start: Instant,
    pub app_start_utc: DateTime<Utc>,
    pub tick_start: Instant,
    pub tick_delta: Duration,
    pub tick_count: u64,
}

impl Time {
    pub fn delta_seconds(&self) -> f32 {
        self.tick_delta.as_secs_f32()
    }

    pub fn tick_start_seconds(&self) -> f32 {
        (self.tick_start - self.app_start).as_secs_f32()
    }
}

#[derive(Debug, SystemParam)]
pub struct CloseApp<'w> {
    app_state: ResMut<'w, AppState>,
}

impl<'w> CloseApp<'w> {
    pub fn request_close(&mut self) {
        *self.app_state = AppState::Exiting;
    }
}
