pub mod block_type;
pub mod terrain;

use std::f32::consts::FRAC_PI_4;

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
    input::Keys,
    render::{
        camera::CameraProjection,
        camera_controller::{
            CameraController,
            CameraControllerState,
        },
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
    world::{
        block_type::BlockTypes,
        terrain::{
            TerrainGenerator,
            TerrainVoxel,
            WorldSeed,
        },
    },
};

pub const CHUNK_SIZE: usize = 32;

#[derive(Clone, Copy, Debug, Default)]
pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder
            .insert_resource(WorldSeed::default())
            .add_plugin(ChunkMeshPlugin::<
                TerrainVoxel,
                GreedyMesher<TerrainVoxel, CHUNK_SIZE>,
                //NaiveMesher,
                CHUNK_SIZE,
            >::default())?
            .add_plugin(ChunkMapPlugin)?
            .add_plugin(ChunkLoaderPlugin::<CHUNK_SIZE>)?
            .add_plugin(FpsCounterPlugin::default())?
            .add_plugin(ChunkGeneratorPlugin::<
                TerrainVoxel,
                TerrainGenerator,
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
    world_seed: Res<WorldSeed>,
    mut commands: Commands,
) {
    commands.insert_resource(TerrainGenerator::new(world_seed.0, &block_types));
}

fn init_player(mut commands: Commands) {
    tracing::debug!("initializing world");

    let chunk_side_length = CHUNK_SIZE as f32;
    let chunk_center = Point3::from(Vector3::repeat(0.5 * chunk_side_length));

    // spawn camera
    let camera_entity = commands
        .spawn((
            Name::new("main_camera"),
            CameraProjection::default(),
            LocalTransform::from(chunk_center + chunk_side_length * Vector3::y()),
            CameraController {
                state: CameraControllerState {
                    yaw: 0.0,
                    pitch: -FRAC_PI_4,
                },
                config: Default::default(),
            },
            ChunkLoader { radius: 8 },
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
