pub mod block_type;
pub mod camera_controller;
pub mod file;
pub mod terrain;

use std::{
    f32::consts::FRAC_PI_4,
    path::PathBuf,
};

use bevy_ecs::{
    entity::Entity,
    name::Name,
    resource::Resource,
    schedule::{
        IntoScheduleConfigs,
        common_conditions::resource_changed,
    },
    system::{
        Commands,
        Query,
        Res,
        ResMut,
    },
};
use color_eyre::eyre::Error;
use nalgebra::{
    Point3,
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
            FpsCounterPlugin,
        },
        mesh::RenderWireframes,
        surface::{
            AttachedCamera,
            ClearColor,
        },
        texture_atlas::{
            AtlasBuilder,
            AtlasSystems,
        },
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
            .add_plugin(FpsCounterPlugin::default())?
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
            >::default())?
            .add_systems(
                schedule::Startup,
                (
                    load_block_types.in_set(AtlasSystems::InsertTextures),
                    create_terrain_generator.after(load_block_types),
                    init_player,
                ),
            )
            .add_systems(
                schedule::Update,
                (
                    show_fps_in_window_title.run_if(resource_changed::<FpsCounter>),
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
    let window = commands
        .spawn((
            Name::new("main_window"),
            WindowConfig {
                title: "SandVox".to_owned(),
            },
            ClearColor(palette::named::LIGHTSKYBLUE.into_format().with_alpha(1.0)),
            AttachedCamera(camera_entity),
        ))
        .id();

    commands.insert_resource(MainWindow(window));
}

#[derive(Clone, Copy, Debug, Resource)]
struct MainWindow(Entity);

fn show_fps_in_window_title(
    fps_counter: Res<FpsCounter>,
    main_window: Res<MainWindow>,
    mut windows: Query<&mut WindowConfig>,
) {
    let mut config = windows.get_mut(main_window.0).unwrap();
    config.title = format!("SandVox ({:.2} fps)", fps_counter.fps);
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
