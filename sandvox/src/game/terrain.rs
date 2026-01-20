use std::time::Instant;

use bevy_ecs::{
    resource::Resource,
    system::Res,
};
use morton_encoding::{
    morton_decode,
    morton_encode,
};
use nalgebra::{
    Point3,
    Vector2,
    Vector3,
};
use rand::{
    Rng,
    SeedableRng,
    distr::{
        Distribution,
        StandardUniform,
    },
};
use rand_xoshiro::Xoroshiro128PlusPlus;
use serde::{
    Deserialize,
    Serialize,
};

use crate::{
    game::{
        CHUNK_SIZE,
        block_type::{
            BlockType,
            BlockTypes,
        },
    },
    render::atlas::AtlasId,
    util::noise::{
        FractalNoise,
        NoiseFn,
        NoiseFnExt,
        PerlinNoise,
        WithAmplitude,
        WithBias,
    },
    voxel::{
        BlockFace,
        Voxel,
        chunk::Chunk,
        chunk_generator::ChunkGenerator,
    },
};

#[derive(Clone, Debug, Default, Resource, Serialize, Deserialize)]
pub struct WorldConfig {
    pub seed: WorldSeed,
    pub bounds: WorldBounds,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct WorldBounds {
    pub min: Vector3<Option<i32>>,
    pub max: Vector3<Option<i32>>,
}

#[derive(Clone, Copy, Debug)]
pub struct TerrainVoxel {
    pub block_type: BlockType,
}

impl Voxel for TerrainVoxel {
    type FetchData = Res<'static, BlockTypes>;
    type Data = BlockTypes;

    fn texture<'w, 's>(&self, face: BlockFace, block_types: &BlockTypes) -> Option<AtlasId> {
        block_types[self.block_type].face_texture(face)
    }

    fn is_opaque<'w, 's>(&self, block_types: &BlockTypes) -> bool {
        let block_type_data = &block_types[self.block_type];
        block_type_data.is_opaque
    }

    fn can_merge<'w, 's>(&self, other: &Self, block_types: &BlockTypes) -> bool {
        let _ = block_types;
        // todo: proper check (e.g. for log textures). this needs to know the face.
        self.block_type == other.block_type
    }
}

#[derive(Debug, Resource)]
pub struct TerrainGenerator {
    // todo: do we need to store the whole world config? probably right? only time will tell...
    world_config: WorldConfig,

    // noises
    surface_height: WithAmplitude<FractalNoise<PerlinNoise>>,
    dirt_depth: WithBias<WithAmplitude<FractalNoise<PerlinNoise>>>,

    // block types used in generation
    air: BlockType,
    dirt: BlockType,
    grass: BlockType,
    stone: BlockType,
    //sand: BlockType,
}

impl TerrainGenerator {
    pub fn new(world_config: &WorldConfig, block_types: &BlockTypes) -> Self {
        // seed a RNG with the world seed so each individual noise function is seeded
        // differently
        let mut rng = Xoroshiro128PlusPlus::seed_from_u64(world_config.seed.0);

        let surface_height =
            FractalNoise::<PerlinNoise>::new(|| rng.random(), 4, 1.0 / 128.0, 2.0, 0.5)
                .with_amplitude(32.0);

        let dirt_depth = FractalNoise::<PerlinNoise>::new(|| rng.random(), 2, 1.0 / 32.0, 2.0, 0.5)
            .with_amplitude(2.0)
            .with_bias(2.0);

        Self {
            world_config: world_config.clone(),
            surface_height,
            dirt_depth,
            air: block_types.lookup("air").unwrap(),
            dirt: block_types.lookup("dirt").unwrap(),
            grass: block_types.lookup("grass").unwrap(),
            stone: block_types.lookup("stone").unwrap(),
            //sand: block_types.lookup("sand").unwrap(),
        }
    }
}

impl ChunkGenerator<TerrainVoxel, CHUNK_SIZE> for TerrainGenerator {
    fn early_discard(&self, position: Point3<i32>) -> bool {
        // todo: should this be an option on the chunk loader instead? (we should still
        // keep this trait method so the chunk generator can opt out of generating a
        // chunk early before dispatching the request to a thread).

        let WorldBounds { min, max } = self.world_config.bounds;

        let is_inside_bounds = (0..3).all(|i| {
            min[i].is_none_or(|min| min <= position[i])
                && max[i].is_none_or(|max| position[i] <= max)
        });

        !is_inside_bounds
    }

    fn generate_chunk(
        &self,
        chunk_position: Point3<i32>,
    ) -> Option<Chunk<TerrainVoxel, CHUNK_SIZE>> {
        let start_time = Instant::now();

        #[derive(Debug, Default)]
        struct Cell {
            surface_height: i64,
            dirt_depth: i64,
        }

        let mut any_blocks = false;
        let chunk_y = chunk_position.y as i64 * CHUNK_SIZE as i64;

        let cells = (0..(CHUNK_SIZE * CHUNK_SIZE))
            .map(|i| {
                let chunk_offset = Vector2::from(morton_decode::<u16, 2>(i as u32));
                let point = chunk_position.xz().cast::<f32>() * CHUNK_SIZE as f32
                    + chunk_offset.cast::<f32>();

                let surface_height = self.surface_height.evaluate_at(point) as i64;
                let dirt_depth = self.dirt_depth.evaluate_at(point) as i64;

                if chunk_y <= surface_height {
                    any_blocks = true;
                }

                Cell {
                    surface_height,
                    dirt_depth,
                }
            })
            .collect::<Vec<_>>();

        let mut chunk = None;

        if any_blocks {
            chunk = Some(Chunk::from_fn(move |point| {
                let cell = &cells[morton_encode(point.xz().into()) as usize];
                let y = chunk_position.y as i64 * CHUNK_SIZE as i64 + point.y as i64;

                let block_type = if y > cell.surface_height {
                    self.air
                }
                else if y == cell.surface_height && cell.dirt_depth >= 1 {
                    self.grass
                }
                else if y < cell.surface_height && y >= cell.surface_height - cell.dirt_depth {
                    self.dirt
                }
                else {
                    self.stone
                };

                TerrainVoxel { block_type }
            }));

            let elapsed = start_time.elapsed();
            tracing::trace!(?chunk_position, ?elapsed, "generated chunk");
        }

        chunk
    }
}

#[derive(
    Clone, Copy, derive_more::Debug, PartialEq, Eq, Hash, Resource, Serialize, Deserialize,
)]
pub struct WorldSeed(#[debug("0x{:x}", self.0)] pub u64);

impl Default for WorldSeed {
    fn default() -> Self {
        // for now this will be fixed, but we might want the default to be random. not
        // sure. you can do either explicitely though.
        Self::FIXED_DEFAULT
    }
}

impl WorldSeed {
    /// Chosen with a fair dice.
    pub const FIXED_DEFAULT: Self = Self(0xc481ec1f222d0691);

    pub fn from_str(seed: &str) -> Self {
        Self(seahash::hash(seed.as_bytes()))
    }
}

impl Distribution<WorldSeed> for StandardUniform {
    fn sample<R: Rng + ?Sized>(&self, rng: &mut R) -> WorldSeed {
        WorldSeed(rng.next_u64())
    }
}

/*
#[derive(Clone, Copy, Debug)]
struct TerrainNoiseParameters {
    temperature: f32,
    humidity: f32,
    continentalness: f32,
    erosion: f32,
}
*/

#[derive(Debug, Resource)]
pub struct TestChunkGenerator {
    stone: BlockType,
}

impl TestChunkGenerator {
    pub fn new(block_types: &BlockTypes) -> Self {
        Self {
            stone: block_types.lookup("stone").unwrap(),
        }
    }
}

impl ChunkGenerator<TerrainVoxel, CHUNK_SIZE> for TestChunkGenerator {
    fn early_discard(&self, position: Point3<i32>) -> bool {
        position != Point3::origin()
    }

    fn generate_chunk(
        &self,
        _chunk_position: Point3<i32>,
    ) -> Option<Chunk<TerrainVoxel, CHUNK_SIZE>> {
        Some(Chunk::from_fn(move |_point| {
            TerrainVoxel {
                block_type: self.stone,
            }
        }))
    }
}

#[cfg(test)]
mod tests {
    use crate::game::terrain::WorldSeed;

    #[test]
    fn world_seed_hashing_is_stable() {
        assert_eq!(
            WorldSeed::from_str("Hello World"),
            WorldSeed(0xbba0b10a3f32e802)
        );
    }
}
