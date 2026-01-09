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
use color_eyre::eyre::Error;
use nalgebra::{
    Point3,
    Vector2,
    Vector3,
};
use noise::{
    NoiseFn,
    Perlin,
};
use winit::{
    application::ApplicationHandler,
    event::StartCause,
    event_loop::{
        ActiveEventLoop,
        ControlFlow,
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
        transform::{
            LocalTransform,
            TransformHierarchyPlugin,
        },
    },
    render::{
        RenderPlugin,
        camera::{
            CameraPlugin,
            CameraProjection,
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
            .add_plugin(WgpuPlugin::default())?
            .add_plugin(RenderPlugin::default())?
            .add_plugin(CameraPlugin)?
            .add_plugin(FlatChunkPlugin::<TestVoxel>::default())?
            .add_systems(schedule::PostStartup, init_world)
            .build();

        Ok(Self { world })
    }

    pub fn run(mut self) -> Result<(), Error> {
        let event_loop = EventLoop::new()?;
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
        self.world
            .run_system_cached_with(handle_event, (event_loop, window_id, event))
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

fn handle_event(
    (InRef(event_loop), In(window_id), In(event)): (
        InRef<ActiveEventLoop>,
        In<WindowId>,
        In<winit::event::WindowEvent>,
    ),
    mut params: ParamSet<(CreateWindows, HandleEvents)>,
) {
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
    ) {
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
                    // todo
                }
                _ => {}
            }
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
