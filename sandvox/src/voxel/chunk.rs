use std::{
    marker::PhantomData,
    ops::Index,
    time::Instant,
};

use bevy_ecs::{
    component::Component,
    entity::Entity,
    lifecycle::HookContext,
    message::{
        Message,
        MessageReader,
    },
    system::{
        Commands,
        Local,
        Populated,
        Res,
        StaticSystemParam,
    },
    world::DeferredWorld,
};
use color_eyre::eyre::Error;
use morton_encoding::{
    morton_decode,
    morton_encode,
};
use nalgebra::Point3;

use crate::{
    ecs::{
        plugin::{
            Plugin,
            WorldBuilder,
        },
        schedule,
    },
    render::mesh::{
        Mesh,
        MeshBuilder,
        MeshPlugin,
    },
    voxel::{
        Voxel,
        mesh::ChunkMesher,
    },
    wgpu::WgpuContext,
};

pub struct VoxelChunkPlugin<V, M, const CHUNK_SIZE: usize> {
    _phantom: PhantomData<(V, M)>,
}

impl<V, M, const CHUNK_SIZE: usize> Default for VoxelChunkPlugin<V, M, CHUNK_SIZE> {
    fn default() -> Self {
        assert!(CHUNK_SIZE.is_power_of_two());

        Self {
            _phantom: PhantomData,
        }
    }
}

impl<V, M, const CHUNK_SIZE: usize> Plugin for VoxelChunkPlugin<V, M, CHUNK_SIZE>
where
    V: Voxel,
    M: ChunkMesher<V, CHUNK_SIZE>,
{
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder
            .add_plugin(MeshPlugin)?
            .add_message::<MeshChunkRequest>()
            .add_systems(schedule::PostUpdate, mesh_chunks::<V, M, CHUNK_SIZE>);

        Ok(())
    }
}

#[derive(Clone, Component)]
#[component(on_add = chunk_added, on_remove = chunk_removed)]
pub struct Chunk<V, const CHUNK_SIZE: usize> {
    pub voxels: Box<[V]>,
}

impl<V, const CHUNK_SIZE: usize> Chunk<V, CHUNK_SIZE> {
    pub fn from_fn(mut f: impl FnMut(Point3<u16>) -> V) -> Self {
        let num_voxels = CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE;
        let mut voxels = Box::new_uninit_slice(num_voxels);

        // fixme: memory leak when f panics
        for (i, voxel) in voxels.iter_mut().enumerate() {
            let point = Point3::from(morton_decode::<u16, 3>(i.try_into().unwrap()));
            voxel.write(f(point));
        }

        let voxels = unsafe { voxels.assume_init() };

        Self { voxels }
    }
}

impl<V, const CHUNK_SIZE: usize> Index<Point3<u16>> for Chunk<V, CHUNK_SIZE> {
    type Output = V;

    fn index(&self, index: Point3<u16>) -> &V {
        &self.voxels[morton_encode(index.into()) as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Message)]
struct MeshChunkRequest {
    entity: Entity,
}

#[derive(Clone, Copy, Debug, Default, Component)]
struct ChunkMeshed;

fn chunk_added(mut world: DeferredWorld, context: HookContext) {
    tracing::debug!(entity = ?context.entity, "chunk added");

    world.write_message(MeshChunkRequest {
        entity: context.entity,
    });
}

fn chunk_removed(mut world: DeferredWorld, context: HookContext) {
    let mut commands = world.commands();
    let mut entity = commands.entity(context.entity);
    entity.try_remove::<ChunkMeshed>();
    entity.try_remove::<Mesh>();
}

fn mesh_chunks<V, M, const CHUNK_SIZE: usize>(
    wgpu: Res<WgpuContext>,
    mut requests: MessageReader<MeshChunkRequest>,
    chunk_data: Populated<&Chunk<V, CHUNK_SIZE>>,
    mut commands: Commands,
    voxel_data: StaticSystemParam<V::Data>,
    mut mesh_builder: Local<MeshBuilder>,
    mut chunk_mesher: Local<M>,
) where
    V: Voxel,
    M: ChunkMesher<V, CHUNK_SIZE>,
{
    for request in requests.read() {
        tracing::debug!(entity = ?request.entity, "meshing chunk");

        if let Ok(chunk) = chunk_data.get(request.entity) {
            let mut entity = commands.entity(request.entity);

            let t_start = Instant::now();
            chunk_mesher.mesh_chunk(&chunk, &mut mesh_builder, &voxel_data);
            let time = t_start.elapsed();
            tracing::debug!(?time, "meshed chunk");

            entity.insert(ChunkMeshed);
            if let Some(mesh) = mesh_builder.finish(&wgpu, "chunk") {
                entity.insert(mesh);
            }

            mesh_builder.clear();
        }
        else {
            tracing::warn!(entity = ?request.entity, "requested chunk to be meshed, but it doesn't have chunk data");
        }
    }
}
