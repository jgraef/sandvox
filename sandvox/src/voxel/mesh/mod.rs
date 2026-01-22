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
    system::{
        Commands,
        Local,
        Populated,
        Res,
        StaticSystemParam,
    },
    world::{
        CommandQueue,
        World,
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
        background_tasks::{
            BackgroundTaskConfig,
            BackgroundTaskPool,
            Task,
            WorldBuilderBackgroundTaskExt,
        },
        plugin::{
            Plugin,
            WorldBuilder,
        },
        schedule,
        workspace::Workspaces,
    },
    render::mesh::{
        MeshBuilder,
        MeshPlugin,
        Vertex,
    },
    voxel::{
        BlockFace,
        Voxel,
        chunk::Chunk,
    },
    wgpu::WgpuContext,
};

pub struct ChunkMeshPlugin<V, M, const CHUNK_SIZE: usize> {
    task_config: BackgroundTaskConfig,
    _phantom: PhantomData<(V, M)>,
}

impl<V, M, const CHUNK_SIZE: usize> ChunkMeshPlugin<V, M, CHUNK_SIZE> {
    pub fn new(task_config: BackgroundTaskConfig) -> Self {
        assert!(CHUNK_SIZE.is_power_of_two());

        Self {
            task_config,
            _phantom: PhantomData,
        }
    }
}

impl<V, M, const CHUNK_SIZE: usize> Default for ChunkMeshPlugin<V, M, CHUNK_SIZE> {
    fn default() -> Self {
        Self::new(Default::default())
    }
}

impl<V, M, const CHUNK_SIZE: usize> Plugin for ChunkMeshPlugin<V, M, CHUNK_SIZE>
where
    V: Voxel,
    M: ChunkMesher<V, CHUNK_SIZE>,
{
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder
            .configure_background_task_queue::<MeshChunkTask<V, M, CHUNK_SIZE>>(self.task_config);

        builder
            .add_plugin(MeshPlugin)?
            .add_systems(schedule::Update, dispatch_chunk_meshing::<V, M, CHUNK_SIZE>);

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Component)]
struct ChunkMeshed;

#[derive(Clone, Copy, Debug, Default, Component)]
struct MeshChunkTaskDispatched;

#[derive(Debug)]
struct MeshChunkTask<V, M, const CHUNK_SIZE: usize>
where
    V: Voxel,
{
    entity: Entity,
    chunk: Chunk<V, CHUNK_SIZE>,
    wgpu: WgpuContext,
    voxel_data: V::Data,
    workspaces: Workspaces<(MeshBuilder, M)>,
}

impl<V, M, const CHUNK_SIZE: usize> Task for MeshChunkTask<V, M, CHUNK_SIZE>
where
    V: Voxel,
    M: ChunkMesher<V, CHUNK_SIZE>,
{
    fn run(self, world_modifications: &mut CommandQueue) {
        let mut workspace = self.workspaces.get();
        let (mesh_builder, chunk_mesher) = &mut *workspace;

        let t_start = Instant::now();
        chunk_mesher.mesh_chunk(&self.chunk, mesh_builder, &self.voxel_data);
        let time = t_start.elapsed();
        tracing::trace!(entity = ?self.entity, ?time, "meshed chunk");

        let mesh = mesh_builder.finish(&self.wgpu, &format!("chunk {:?}", self.entity));
        mesh_builder.clear();

        world_modifications.push(move |world: &mut World| {
            let mut commands = world.commands();
            let mut entity = commands.entity(self.entity);
            entity.remove::<MeshChunkTaskDispatched>();
            entity.insert(ChunkMeshed);
            if let Some(mesh) = mesh {
                entity.insert(mesh);
            }
        });
    }
}

fn dispatch_chunk_meshing<V, M, const CHUNK_SIZE: usize>(
    wgpu: Res<WgpuContext>,
    background_tasks: Res<BackgroundTaskPool>,
    chunks: Populated<
        (Entity, &Chunk<V, CHUNK_SIZE>),
        (
            Or<(Without<ChunkMeshed>, Changed<Chunk<V, CHUNK_SIZE>>)>,
            Without<MeshChunkTaskDispatched>,
        ),
    >,
    voxel_data: StaticSystemParam<V::FetchData>,
    workspaces: Local<Workspaces<(MeshBuilder, M)>>,
    mut commands: Commands,
) where
    V: Voxel,
    M: ChunkMesher<V, CHUNK_SIZE>,
{
    let voxel_data: V::Data = (&*voxel_data).into();
    background_tasks.push_tasks(chunks.iter().map(|(entity, chunk)| {
        commands.entity(entity).insert(MeshChunkTaskDispatched);

        MeshChunkTask {
            entity,
            chunk: chunk.clone(),
            wgpu: wgpu.clone(),
            voxel_data: voxel_data.clone(),
            workspaces: workspaces.clone(),
        }
    }));
}

pub trait ChunkMesher<V, const CHUNK_SIZE: usize>: Send + Sync + Default + 'static
where
    V: Voxel,
{
    fn mesh_chunk<'w, 's>(
        &mut self,
        chunk: &Chunk<V, CHUNK_SIZE>,
        mesh_builder: &mut MeshBuilder,
        data: &V::Data,
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
    fn uvs(&self, face: BlockFace) -> [Point2<u16>; 4] {
        let dx = self.ij1.x - self.ij0.x;
        let dy = self.ij1.y - self.ij0.y;

        match face {
            BlockFace::Left => [[dx, 0], [0, 0], [0, dy], [dx, dy]],
            BlockFace::Right | BlockFace::Down | BlockFace::Up => {
                [[0, 0], [dx, 0], [dx, dy], [0, dy]]
            }
            BlockFace::Front => [[0, dy], [dx, dy], [dx, 0], [0, 0]],
            BlockFace::Back => [[dx, dy], [0, dy], [0, 0], [dx, 0]],
        }
        .map(Into::into)
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

        let uvs = self.uvs(face);

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
