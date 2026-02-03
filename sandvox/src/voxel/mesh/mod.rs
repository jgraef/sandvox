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
    resource::Resource,
    system::{
        Commands,
        Local,
        Populated,
        Res,
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
        MeshPipelineLayout,
        MeshPlugin,
        Vertex,
    },
    voxel::{
        BlockFace,
        Voxel,
        VoxelData,
        chunk::{
            Chunk,
            ChunkShape,
        },
        chunk_map::ChunkStatistics,
    },
    wgpu::WgpuContext,
};

pub struct ChunkMeshPlugin<V, S, D, M> {
    task_config: BackgroundTaskConfig,
    _phantom: PhantomData<fn() -> (V, S, D, M)>,
}

impl<V, S, D, M> ChunkMeshPlugin<V, S, D, M> {
    pub fn new(task_config: BackgroundTaskConfig) -> Self {
        Self {
            task_config,
            _phantom: PhantomData,
        }
    }
}

impl<V, S, D, M> Default for ChunkMeshPlugin<V, S, D, M> {
    fn default() -> Self {
        Self::new(Default::default())
    }
}

impl<V, S, D, M> Plugin for ChunkMeshPlugin<V, S, D, M>
where
    V: Voxel,
    S: ChunkShape,
    D: Resource + Clone + VoxelData<V>,
    M: ChunkMesher<V, S>,
{
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder.configure_background_task_queue::<MeshChunkTask<V, S, D, M>>(self.task_config);

        builder
            .add_plugin(MeshPlugin)?
            .add_systems(schedule::Update, dispatch_chunk_meshing::<V, S, D, M>);

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Component)]
struct ChunkMeshed;

#[derive(Clone, Copy, Debug, Default, Component)]
struct MeshChunkTaskDispatched;

#[derive(Debug)]
struct MeshChunkTask<V, S, D, M>
where
    V: Voxel,
{
    entity: Entity,
    chunk: Chunk<V, S>,
    wgpu: WgpuContext,
    mesh_bind_group_layout: wgpu::BindGroupLayout,
    voxel_data: D,
    workspaces: Workspaces<(MeshBuilder, M)>,
}

impl<V, S, D, M> Task for MeshChunkTask<V, S, D, M>
where
    V: Voxel,
    S: ChunkShape,
    M: ChunkMesher<V, S>,
    D: VoxelData<V> + Send + Sync + 'static,
{
    fn run(self, world_modifications: &mut CommandQueue) {
        let mut workspace = self
            .workspaces
            .get_or_init(|| (MeshBuilder::default(), M::new(self.chunk.shape())));

        let (mesh_builder, chunk_mesher) = &mut *workspace;

        let t_start = Instant::now();
        chunk_mesher.mesh_chunk(&self.chunk, mesh_builder, &self.voxel_data);
        let time = t_start.elapsed();
        tracing::trace!(entity = ?self.entity, ?time, "meshed chunk");

        let mesh = mesh_builder.finish(
            &self.wgpu,
            &format!("chunk {:?}", self.entity),
            &self.mesh_bind_group_layout,
        );
        mesh_builder.clear();

        world_modifications.push(move |world: &mut World| {
            if let Some(mesh) = &mesh {
                let mut chunk_statistics = world.resource_mut::<ChunkStatistics>();
                chunk_statistics.num_chunks_meshed += 1;
                chunk_statistics.bytes_chunks_meshed += mesh.byte_size();
            }

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

fn dispatch_chunk_meshing<V, S, D, M>(
    wgpu: Res<WgpuContext>,
    background_tasks: Res<BackgroundTaskPool>,
    chunks: Populated<
        (Entity, &Chunk<V, S>),
        (
            Or<(Without<ChunkMeshed>, Changed<Chunk<V, S>>)>,
            Without<MeshChunkTaskDispatched>,
        ),
    >,
    voxel_data: Res<D>,
    workspaces: Local<Workspaces<(MeshBuilder, M)>>,
    mesh_layout: Res<MeshPipelineLayout>,
    mut commands: Commands,
) where
    V: Voxel,
    S: ChunkShape,
    D: Resource + Clone + VoxelData<V> + Send + Sync + 'static,
    M: ChunkMesher<V, S>,
{
    background_tasks.push_tasks(chunks.iter().map(|(entity, chunk)| {
        commands.entity(entity).insert(MeshChunkTaskDispatched);

        MeshChunkTask {
            entity,
            chunk: chunk.clone(),
            wgpu: wgpu.clone(),
            voxel_data: voxel_data.clone(),
            workspaces: workspaces.clone(),
            mesh_bind_group_layout: mesh_layout.mesh_bind_group_layout.clone(),
        }
    }));
}

pub trait ChunkMesher<V, S>: Send + Sync + 'static
where
    V: Voxel,
    S: ChunkShape,
{
    fn new(shape: &S) -> Self;

    fn mesh_chunk<D>(&mut self, chunk: &Chunk<V, S>, mesh_builder: &mut MeshBuilder, data: &D)
    where
        D: VoxelData<V>;
}

#[derive(Clone, Copy, Debug)]
pub struct UnorientedQuad {
    pub ij0: Point2<u16>,
    pub ij1: Point2<u16>,
    pub k: u16,
}

impl UnorientedQuad {
    #[inline]
    fn xy_vertices(&self) -> [Point3<u16>; 4] {
        [
            [self.ij0.x, self.ij0.y, self.k],
            [self.ij1.x, self.ij0.y, self.k],
            [self.ij1.x, self.ij1.y, self.k],
            [self.ij0.x, self.ij1.y, self.k],
        ]
        .map(Into::into)
    }

    #[inline]
    fn zy_vertices(&self) -> [Point3<u16>; 4] {
        [
            [self.k, self.ij1.y, self.ij0.x],
            [self.k, self.ij1.y, self.ij1.x],
            [self.k, self.ij0.y, self.ij1.x],
            [self.k, self.ij0.y, self.ij0.x],
        ]
        .map(Into::into)
    }

    #[inline]
    fn xz_vertices(&self) -> [Point3<u16>; 4] {
        [
            [self.ij0.x, self.k, self.ij1.y],
            [self.ij1.x, self.k, self.ij1.y],
            [self.ij1.x, self.k, self.ij0.y],
            [self.ij0.x, self.k, self.ij0.y],
        ]
        .map(Into::into)
    }

    #[inline]
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
                padding: 0,
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
