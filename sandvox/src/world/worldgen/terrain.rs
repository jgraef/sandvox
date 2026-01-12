use std::time::Instant;

use bevy_ecs::resource::Resource;
use nalgebra::Point3;
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
        worldgen::noise::{
            FractalNoise,
            NoiseFn,
            NoiseFnExt,
            PerlinNoise,
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
        position.y == 0 && position.x.abs() <= 4 && position.z.abs() <= 4
    }

    fn generate_chunk(
        &self,
        _workspace: &mut Self::Workspace,
        chunk_position: Point3<i32>,
    ) -> Option<Chunk<TerrainVoxel, CHUNK_SIZE>> {
        let start_time = Instant::now();

        let chunk_side_length = CHUNK_SIZE as f32;
        let chunk_position = chunk_position.cast::<f32>();

        let mut rng = Xoroshiro128PlusPlus::seed_from_u64(self.seed);

        /*let noise = rng
        .random::<PerlinNoise>()
        .with_frequency(1.0 / 16.0)
        */

        let noise = FractalNoise::<PerlinNoise, 6>::new(|| rng.random(), 1.0 / 128.0, 2.0, 0.5);

        let height = noise.with_amplitude(16.0).with_bias(16.0);

        let chunk = Chunk::from_fn(move |point| {
            let point = point.cast::<f32>() + chunk_side_length * chunk_position.coords;

            let height = height.evaluate_at(point);

            let block_type = if point.y <= height {
                self.stone
            }
            else {
                self.air
            };

            TerrainVoxel { block_type }
        });

        let elapsed = start_time.elapsed();
        tracing::debug!(?chunk_position, ?elapsed, "generated chunk");

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

#[derive(Clone, Copy, Debug)]
struct TerrainNoiseParameters {
    temperature: f32,
    humidity: f32,
    continentalness: f32,
    erosion: f32,
}
