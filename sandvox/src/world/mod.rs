pub mod block_type;

use bevy_ecs::{
    name::Name,
    schedule::IntoScheduleConfigs,
    system::{
        Commands,
        Res,
        ResMut,
    },
};
use color_eyre::eyre::Error;
use nalgebra::{
    Point3,
    Vector3,
};
use noise::{
    NoiseFn,
    Perlin,
};

use crate::{
    app::Window,
    ecs::{
        plugin::{
            Plugin,
            WorldBuilder,
        },
        schedule,
        transform::LocalTransform,
    },
    render::{
        camera::CameraProjection,
        camera_controller::CameraController,
        surface::{
            AttachedCamera,
            ClearColor,
        },
        texture_atlas::{
            AtlasBuilder,
            AtlasId,
            AtlasSystems,
        },
    },
    voxel::{
        Voxel,
        flat::{
            CHUNK_SIDE_LENGTH,
            FlatChunk,
            FlatChunkPlugin,
        },
    },
    world::block_type::{
        BlockType,
        BlockTypes,
    },
};

#[derive(Clone, Copy, Debug, Default)]
pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder
            .add_plugin(FlatChunkPlugin::<TerrainVoxel>::default())?
            .add_systems(
                schedule::Startup,
                (
                    load_block_types.in_set(AtlasSystems::InsertTextures),
                    init_world.after(load_block_types),
                ),
            );

        Ok(())
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TerrainVoxel {
    pub block_type: BlockType,
}

impl Voxel for TerrainVoxel {
    type SystemParam = Res<'static, BlockTypes>;

    fn texture(&self, block_types: &mut Res<BlockTypes>) -> Option<AtlasId> {
        let block_type_data = &block_types[self.block_type];
        block_type_data.texture_id
    }
}

fn load_block_types(mut atlas_builder: ResMut<AtlasBuilder>, mut commands: Commands) {
    let block_types = BlockTypes::load("assets/blocks.toml", &mut atlas_builder).unwrap();
    commands.insert_resource(block_types);
}

fn init_world(block_types: Res<BlockTypes>, mut commands: Commands) {
    tracing::debug!("initializing world");

    let chunk_side_length = CHUNK_SIDE_LENGTH as f32;
    let chunk_center = Point3::from(Vector3::repeat(0.5 * chunk_side_length));

    // spawn chunk
    commands.spawn((
        {
            let dirt = block_types.lookup("dirt").unwrap();
            let stone = block_types.lookup("stone").unwrap();

            let noise = Perlin::new(1312);
            let scaling = 1.0 / chunk_side_length;

            FlatChunk::from_fn(move |point| {
                let value = noise.get((point.cast::<f32>() * scaling).cast::<f64>().into());
                let block_type = if value > 0.0 { dirt } else { stone };
                TerrainVoxel { block_type }
            })
        },
        LocalTransform::from(Point3::origin()),
    ));

    // spawn camera
    let camera_entity = commands
        .spawn((
            Name::new("main_camera"),
            CameraProjection::default(),
            LocalTransform::look_at(
                &(chunk_center - chunk_side_length * Vector3::z()),
                &chunk_center,
                &Vector3::y(),
            ),
            CameraController::default(),
        ))
        .id();

    // spawn window
    commands.spawn((
        Name::new("main_window"),
        Window {
            title: "SandVox".to_owned(),
        },
        ClearColor::default(),
        AttachedCamera(camera_entity),
    ));
}
