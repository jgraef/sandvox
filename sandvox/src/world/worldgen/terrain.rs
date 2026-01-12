use bevy_ecs::resource::Resource;
use nalgebra::Point3;
use noise::{
    NoiseFn,
    Perlin,
};
use rand::{
    Rng,
    SeedableRng,
};
use rand_xoshiro::Xoroshiro128PlusPlus;

use crate::{
    voxel::{
        chunk::Chunk,
        chunk_generator::ChunkGenerator,
    },
    world::{
        CHUNK_SIZE,
        TerrainVoxel,
        block_type::{
            BlockType,
            BlockTypes,
        },
    },
};

#[derive(Debug)]
pub struct TerrainGenerator {
    seed: u64,

    // block types used in generation
    air: BlockType,
    dirt: BlockType,
    stone: BlockType,
    sand: BlockType,
}

impl TerrainGenerator {
    pub fn new(seed: u64, block_types: &BlockTypes) -> Self {
        Self {
            seed,
            air: block_types.lookup("air").unwrap(),
            dirt: block_types.lookup("dirt").unwrap(),
            stone: block_types.lookup("stone").unwrap(),
            sand: block_types.lookup("sand").unwrap(),
        }
    }
}

impl ChunkGenerator<TerrainVoxel, CHUNK_SIZE> for TerrainGenerator {
    type Workspace = ();

    fn create_workspace(&self) -> Self::Workspace {
        ()
    }

    fn filter(&self, position: Point3<i32>) -> bool {
        position.x >= -1 && position.x <= 1 && position.y == 0 && position.z == 0
    }

    fn generate_chunk(
        &self,
        _workspace: &mut Self::Workspace,
        chunk_position: Point3<i32>,
    ) -> Option<Chunk<TerrainVoxel, CHUNK_SIZE>> {
        tracing::debug!(?chunk_position, "generating chunk");

        let chunk_side_length = CHUNK_SIZE as f32;
        let chunk_position = chunk_position.cast::<f32>();

        let mut rng = Xoroshiro128PlusPlus::seed_from_u64(self.seed);

        let noise = Perlin::new(rng.random());
        let frequency = 1.0 / chunk_side_length;
        let amplitude = 10.0;
        let offset = 15.0;

        let chunk = Chunk::from_fn(move |point| {
            let point = point.cast::<f32>() + chunk_side_length * chunk_position.coords;

            let height = amplitude
                * noise.get((point.xz() * frequency).cast::<f64>().into()) as f32
                + offset;

            let block_type = if point.y <= height {
                self.stone
            }
            else {
                self.air
            };

            TerrainVoxel { block_type }
        });

        Some(chunk)
    }
}

#[derive(Clone, Copy, Debug, Resource)]
pub struct WorldSeed(pub u64);

impl Default for WorldSeed {
    fn default() -> Self {
        // chosen with a fair dice
        Self(0xc481ec1f222d0691)
    }
}
