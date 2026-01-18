pub mod block_type;
pub mod camera_controller;
pub mod file;
pub mod terrain;

use std::{
    f32::consts::FRAC_PI_4,
    fmt::Write,
    path::PathBuf,
};

use bevy_ecs::{
    component::Component,
    name::Name,
    query::With,
    resource::Resource,
    schedule::{
        IntoScheduleConfigs,
        SystemCondition,
        common_conditions::{
            any_with_component,
            resource_changed,
        },
    },
    system::{
        Commands,
        Res,
        ResMut,
        Single,
    },
};
use color_eyre::eyre::Error;
use nalgebra::{
    Point3,
    Vector2,
    Vector3,
};
use palette::WithAlpha;
use serde::{
    Deserialize,
    Serialize,
};
use winit::keyboard::KeyCode;

use crate::{
    app::{
        CloseApp,
        WindowConfig,
    },
    ecs::{
        background_tasks::BackgroundTaskConfig,
        plugin::{
            Plugin,
            WorldBuilder,
        },
        schedule,
        transform::LocalTransform,
    },
    game::{
        block_type::BlockTypes,
        camera_controller::{
            CameraController,
            CameraControllerConfig,
            CameraControllerPlugin,
            CameraControllerState,
        },
        file::WorldFile,
        terrain::{
            TerrainGenerator,
            TerrainVoxel,
            WorldConfig,
        },
    },
    input::Keys,
    render::{
        camera::CameraProjection,
        fps_counter::{
            FpsCounter,
            FpsCounterConfig,
        },
        mesh::RenderWireframes,
        surface::{
            AttachedCamera,
            ClearColor,
        },
        text::{
            Text,
            TextSize,
        },
        texture_atlas::{
            AtlasBuilder,
            AtlasSystems,
        },
    },
    ui::{
        UiSurface,
        layout::Style,
    },
    voxel::{
        chunk_generator::ChunkGeneratorPlugin,
        chunk_map::ChunkMapPlugin,
        loader::{
            ChunkLoader,
            ChunkLoaderPlugin,
        },
        mesh::{
            ChunkMeshPlugin,
            greedy_quads::GreedyMesher,
        },
    },
};

pub const CHUNK_SIZE: usize = 32;

#[derive(Clone, Debug, Default)]
pub struct GamePlugin {
    pub game_config: GameConfig,
    pub init_world: InitWorld,
}

#[derive(Clone, Debug, Resource, Serialize, Deserialize)]
pub struct GameConfig {
    #[serde(default = "default_chunk_distance")]
    pub chunk_load_distance: u32,

    #[serde(default = "default_chunk_distance")]
    pub chunk_render_distance: u32,

    #[serde(default)]
    pub chunk_generator_config: BackgroundTaskConfig,

    #[serde(default)]
    pub camera_controller: CameraControllerConfig,
}

fn default_chunk_distance() -> u32 {
    4
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            chunk_load_distance: default_chunk_distance(),
            chunk_render_distance: default_chunk_distance(),
            chunk_generator_config: Default::default(),
            camera_controller: Default::default(),
        }
    }
}

#[derive(Clone, Debug, Resource)]
pub enum InitWorld {
    Load {
        world_file: PathBuf,
    },
    Create {
        world_config: WorldConfig,
        world_file: Option<PathBuf>,
    },
}

impl Default for InitWorld {
    fn default() -> Self {
        Self::Create {
            world_config: Default::default(),
            world_file: None,
        }
    }
}

impl Plugin for GamePlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        match &self.init_world {
            InitWorld::Load { world_file } => {
                let world_file = WorldFile::open(world_file)?;

                builder
                    .insert_resource(world_file.world_config().clone())
                    .insert_resource(world_file);
            }
            InitWorld::Create {
                world_config,
                world_file,
            } => {
                builder.insert_resource(world_config.clone());

                if let Some(world_file) = world_file {
                    let world_file = WorldFile::create(&world_file, world_config.clone())?;
                    builder.insert_resource(world_file);
                }
            }
        }

        builder
            .insert_resource(self.game_config.clone())
            .add_plugin(CameraControllerPlugin)?
            .add_plugin(ChunkMeshPlugin::<
                TerrainVoxel,
                GreedyMesher<TerrainVoxel, CHUNK_SIZE>,
                //NaiveMesher,
                CHUNK_SIZE,
            >::default())?
            .add_plugin(ChunkMapPlugin)?
            .add_plugin(ChunkLoaderPlugin::<CHUNK_SIZE>)?
            .add_plugin(ChunkGeneratorPlugin::<
                TerrainVoxel,
                TerrainGenerator,
                //TestChunkGenerator,
                CHUNK_SIZE,
            >::new(self.game_config.chunk_generator_config))?
            .add_systems(
                schedule::Startup,
                (
                    load_block_types.in_set(AtlasSystems::InsertTextures),
                    create_terrain_generator.after(load_block_types),
                    init_player,
                    init_debug_overlay,
                ),
            )
            .add_systems(
                schedule::Update,
                (
                    update_debug_overlay.run_if(
                        resource_changed::<FpsCounter>.and(any_with_component::<DebugOverlay>),
                    ),
                    handle_keys.run_if(resource_changed::<Keys>),
                ),
            );

        Ok(())
    }
}

fn load_block_types(mut atlas_builder: ResMut<AtlasBuilder>, mut commands: Commands) {
    let block_types = BlockTypes::load("assets/blocks.toml", &mut atlas_builder).unwrap();
    commands.insert_resource(block_types);
}

fn create_terrain_generator(
    block_types: Res<BlockTypes>,
    world_config: Res<WorldConfig>,
    mut commands: Commands,
) {
    commands.insert_resource(TerrainGenerator::new(&world_config, &block_types));
    //commands.insert_resource(TestChunkGenerator::new(&block_types));
}

fn init_player(config: Res<GameConfig>, mut commands: Commands) {
    tracing::debug!("initializing world");

    let chunk_side_length = CHUNK_SIZE as f32;
    let chunk_center = Point3::from(Vector3::repeat(0.5 * chunk_side_length));

    // spawn camera
    let camera_entity = commands
        .spawn((
            Name::new("main_camera"),
            CameraProjection::new(
                CameraProjection::DEFAULT_FOVY,
                config.chunk_render_distance as f32 * CHUNK_SIZE as f32,
            ),
            LocalTransform::from(chunk_center + chunk_side_length * Vector3::y()),
            CameraController {
                state: CameraControllerState {
                    yaw: 0.0,
                    pitch: -FRAC_PI_4,
                },
                config: config.camera_controller.clone(),
            },
            ChunkLoader {
                radius: Vector3::repeat(config.chunk_load_distance),
            },
        ))
        .id();

    // spawn window
    commands.spawn((
        Name::new("main_window"),
        WindowConfig {
            title: "SandVox".to_owned(),
        },
        ClearColor(palette::named::LIGHTSKYBLUE.into_format().with_alpha(1.0)),
        AttachedCamera(camera_entity),
    ));
}

#[derive(Clone, Copy, Debug, Default, Component)]
struct DebugOverlay;

fn init_debug_overlay(mut _fps_counter_config: ResMut<FpsCounterConfig>, mut commands: Commands) {
    //fps_counter_config.measurement_inverval = Duration::from_millis(100);
    //commands.spawn((Text::default(), TextSize { height: 2.0 }, DebugOverlay));

    commands
        .spawn((
            UiSurface {
                size: Vector2::new(400.0, 400.0),
            },
            {
                let mut style = Style::default();
                style.display = taffy::style::Display::Block;
                style
            },
        ))
        .with_children(|spawner| {
            /*spawner.spawn((
                Text::from("Hello World!"),
                TextSize { height: 2.0 },
                Style::default(),
                LeafMeasure::default(),
            ));*/

            spawner.spawn((
                Text::default(),
                TextSize { height: 2.0 },
                Style::default(),
                DebugOverlay,
            ));
        });
}

fn update_debug_overlay(
    fps_counter: Res<FpsCounter>,
    mut debug_overlay: Single<&mut Text, With<DebugOverlay>>,
) {
    debug_overlay.text.clear();
    write!(&mut debug_overlay.text, "FPS: {:.1}", fps_counter.fps).unwrap();
}

fn handle_keys(
    keys: Res<Keys>,
    render_wireframes: Option<Res<RenderWireframes>>,
    mut close_app: CloseApp,
    mut commands: Commands,
) {
    if keys.just_pressed.contains(&KeyCode::F6) {
        if render_wireframes.is_none() {
            commands.insert_resource(RenderWireframes);
        }
        else {
            commands.remove_resource::<RenderWireframes>();
        }
    }

    if keys.just_pressed.contains(&KeyCode::Escape) {
        close_app.request_close();
    }
}
