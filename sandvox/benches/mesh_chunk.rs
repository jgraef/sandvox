use std::hint::black_box;

use criterion::{
    BenchmarkGroup,
    Criterion,
    criterion_group,
    criterion_main,
    measurement::WallTime,
};
use nalgebra::Point3;
use rand::{
    Rng,
    rng,
};
use sandvox::{
    render::mesh::MeshBuilder,
    voxel::{
        BlockFace,
        Voxel,
        VoxelData,
        chunk::{
            Chunk,
            ChunkShape,
            LinearShape,
            MortonShape,
        },
        chunk_generator::ChunkGenerator,
        mesh::{
            ChunkMesher,
            greedy_quads::GreedyMesher,
            naive::NaiveMesher,
        },
    },
};

#[derive(Clone, Copy, Debug)]
struct TestVoxel {
    id: u8,
    _data: u8,
}

impl Voxel for TestVoxel {}

#[derive(Debug)]
struct TestTerrainGenerator;

impl<S> ChunkGenerator<TestVoxel, S> for TestTerrainGenerator
where
    S: ChunkShape,
{
    fn generate_chunk(&self, _position: Point3<i32>, shape: S) -> Option<Chunk<TestVoxel, S>> {
        let mut rng = rng();

        Some(Chunk::from_fn(shape, |_position| {
            TestVoxel {
                id: rng.random(),
                _data: rng.random(),
            }
        }))
    }
}

impl VoxelData<TestVoxel> for () {
    #[inline]
    fn texture(&self, voxel: &TestVoxel, _face: BlockFace) -> Option<u32> {
        (voxel.id != 0).then(|| voxel.id.into())
    }

    #[inline]
    fn is_opaque(&self, voxel: &TestVoxel) -> bool {
        voxel.id & 0x80 == 0
    }

    #[inline]
    fn can_merge(&self, first: &TestVoxel, second: &TestVoxel) -> bool {
        first.id == second.id
    }
}

fn bench_with_shape<S>(group: &mut BenchmarkGroup<WallTime>, shape: S, shape_name: &str)
where
    S: ChunkShape,
{
    let chunks = std::iter::from_fn(|| {
        Some(
            TestTerrainGenerator
                .generate_chunk(Point3::origin(), shape.clone())
                .unwrap(),
        )
    })
    .take(1000)
    .collect::<Vec<_>>();

    let mut chunks = chunks.iter().cycle();
    let mut mesh_builder = MeshBuilder::default();

    let mut chunk_mesher = GreedyMesher::new(&shape);
    group.bench_function(format!("greedy/{shape_name}"), |b| {
        b.iter(|| {
            chunk_mesher.mesh_chunk(black_box(chunks.next().unwrap()), &mut mesh_builder, &());
            mesh_builder.clear();
        })
    });

    let mut chunk_mesher = <NaiveMesher as ChunkMesher<TestVoxel, S>>::new(&shape);
    group.bench_function(format!("naive/{shape_name}"), |b| {
        b.iter(|| {
            chunk_mesher.mesh_chunk(black_box(chunks.next().unwrap()), &mut mesh_builder, &());
            mesh_builder.clear();
        })
    });
}

fn bench_chunk_meshing(c: &mut Criterion) {
    let mut group = c.benchmark_group("mesh_chunk");

    bench_with_shape(&mut group, MortonShape::<32>, "morton");
    bench_with_shape(&mut group, LinearShape::<32>, "linear");

    group.finish();
}

criterion_group!(benches, bench_chunk_meshing);
criterion_main!(benches);
