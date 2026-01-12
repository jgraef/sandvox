pub mod greedy_quads;
pub mod naive;

use std::{
    marker::PhantomData,
    time::Instant,
};

use bevy_ecs::{
    component::Component,
    entity::Entity,
    query::{
        Changed,
        Or,
        Without,
    },
    schedule::IntoScheduleConfigs,
    system::{
        Commands,
        Local,
        Populated,
        Res,
        StaticSystemParam,
        SystemParam,
    },
};
use color_eyre::eyre::Error;
use nalgebra::{
    Point2,
    Point3,
    Vector3,
    Vector4,
};

use crate::{
    ecs::{
        plugin::{
            Plugin,
            WorldBuilder,
        },
        schedule,
    },
    render::{
        RenderSystems,
        mesh::{
            MeshBuilder,
            MeshPlugin,
            Vertex,
        },
    },
    voxel::{
        BlockFace,
        Voxel,
        chunk::Chunk,
    },
    wgpu::WgpuContext,
};

pub struct ChunkMeshPlugin<V, M, const CHUNK_SIZE: usize> {
    _phantom: PhantomData<(V, M)>,
}

impl<V, M, const CHUNK_SIZE: usize> Default for ChunkMeshPlugin<V, M, CHUNK_SIZE> {
    fn default() -> Self {
        assert!(CHUNK_SIZE.is_power_of_two());

        Self {
            _phantom: PhantomData,
        }
    }
}

impl<V, M, const CHUNK_SIZE: usize> Plugin for ChunkMeshPlugin<V, M, CHUNK_SIZE>
where
    V: Voxel,
    M: ChunkMesher<V, CHUNK_SIZE>,
{
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder.add_plugin(MeshPlugin)?.add_systems(
            schedule::Render,
            mesh_chunks::<V, M, CHUNK_SIZE>.before(RenderSystems::RenderFrame),
        );

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Component)]
struct ChunkMeshed;

fn mesh_chunks<V, M, const CHUNK_SIZE: usize>(
    wgpu: Res<WgpuContext>,
    chunks: Populated<
        (Entity, &Chunk<V, CHUNK_SIZE>),
        Or<(Without<ChunkMeshed>, Changed<Chunk<V, CHUNK_SIZE>>)>,
    >,
    voxel_data: StaticSystemParam<V::Data>,
    mut commands: Commands,
    mut mesh_builder: Local<MeshBuilder>,
    mut chunk_mesher: Local<M>,
) where
    V: Voxel,
    M: ChunkMesher<V, CHUNK_SIZE>,
{
    // todo: do this in a background thread, just like chunk generation works
    // for now we'll just limit how many we do per frame
    const MAX_CHUNKS_MESHED_PER_FRAME: usize = 64;

    let mut num_meshed = 0;

    for (entity, chunk) in &chunks {
        tracing::debug!(?entity, "meshing chunk");

        let mut entity = commands.entity(entity);

        let t_start = Instant::now();
        chunk_mesher.mesh_chunk(&chunk, &mut mesh_builder, &voxel_data);
        let time = t_start.elapsed();
        tracing::debug!(?time, "meshed chunk");

        entity.insert(ChunkMeshed);
        if let Some(mesh) = mesh_builder.finish(&wgpu, "chunk") {
            entity.insert(mesh);
            num_meshed += 1;
        }

        mesh_builder.clear();

        if num_meshed >= MAX_CHUNKS_MESHED_PER_FRAME {
            break;
        }
    }
}

pub trait ChunkMesher<V, const CHUNK_SIZE: usize>: Send + Sync + Default + 'static
where
    V: Voxel,
{
    fn mesh_chunk<'w, 's>(
        &mut self,
        chunk: &Chunk<V, CHUNK_SIZE>,
        mesh_builder: &mut MeshBuilder,
        data: &<V::Data as SystemParam>::Item<'w, 's>,
    );
}

#[derive(Clone, Copy, Debug)]
pub struct UnorientedQuad {
    pub ij0: Point2<u16>,
    pub ij1: Point2<u16>,
    pub k: u16,
}

impl UnorientedQuad {
    #[inline(always)]
    fn xy_vertices(&self) -> [Point3<u16>; 4] {
        [
            [self.ij0.x, self.ij0.y, self.k],
            [self.ij1.x, self.ij0.y, self.k],
            [self.ij1.x, self.ij1.y, self.k],
            [self.ij0.x, self.ij1.y, self.k],
        ]
        .map(Into::into)
    }

    #[inline(always)]
    fn zy_vertices(&self) -> [Point3<u16>; 4] {
        [
            [self.k, self.ij1.y, self.ij0.x],
            [self.k, self.ij1.y, self.ij1.x],
            [self.k, self.ij0.y, self.ij1.x],
            [self.k, self.ij0.y, self.ij0.x],
        ]
        .map(Into::into)
    }

    #[inline(always)]
    fn xz_vertices(&self) -> [Point3<u16>; 4] {
        [
            [self.ij0.x, self.k, self.ij1.y],
            [self.ij1.x, self.k, self.ij1.y],
            [self.ij1.x, self.k, self.ij0.y],
            [self.ij0.x, self.k, self.ij0.y],
        ]
        .map(Into::into)
    }

    #[inline(always)]
    fn uvs(&self) -> [Point2<u16>; 4] {
        let dx = self.ij1.x - self.ij0.x;
        let dy = self.ij1.y - self.ij0.y;

        //[[0, 0], [dx, 0], [dx, dy], [0, dy]]

        // pretty sure this is the right way. y is flipped
        [[0, dy], [dx, dy], [dx, 0], [0, 0]].map(Into::into)
    }

    pub fn mesh(&self, face: BlockFace, texture_id: u32) -> QuadMesh {
        let (vertices, normal, indices, offset) = match face {
            BlockFace::Left => {
                (
                    self.zy_vertices(),
                    -Vector4::x(),
                    FRONT_INDICES,
                    Vector3::zeros(),
                )
            }
            BlockFace::Right => (self.zy_vertices(), Vector4::x(), BACK_INDICES, Vector3::x()),
            BlockFace::Down => {
                (
                    self.xz_vertices(),
                    -Vector4::y(),
                    FRONT_INDICES,
                    Vector3::zeros(),
                )
            }
            BlockFace::Up => (self.xz_vertices(), Vector4::y(), BACK_INDICES, Vector3::y()),
            BlockFace::Front => {
                (
                    self.xy_vertices(),
                    -Vector4::z(),
                    FRONT_INDICES,
                    Vector3::zeros(),
                )
            }
            BlockFace::Back => (self.xy_vertices(), Vector4::z(), BACK_INDICES, Vector3::z()),
        };

        let uvs = self.uvs();

        let vertices = std::array::from_fn::<_, 4, _>(|i| {
            Vertex {
                position: (vertices[i].cast() + offset).to_homogeneous(),
                normal,
                uv: Point2::from(uvs[i]).cast(),
                texture_id,
            }
        });

        QuadMesh {
            vertices,
            faces: indices,
        }
    }
}

pub const FRONT_INDICES: [[u32; 3]; 2] = [[0, 1, 2], [0, 2, 3]];
pub const BACK_INDICES: [[u32; 3]; 2] = [[2, 1, 0], [3, 2, 0]];

#[derive(Clone, Copy, Debug)]
pub struct QuadMesh {
    pub vertices: [Vertex; 4],
    pub faces: [[u32; 3]; 2],
}
