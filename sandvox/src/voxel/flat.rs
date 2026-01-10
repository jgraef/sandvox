use std::{
    marker::PhantomData,
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
use morton_encoding::morton_decode;
use nalgebra::{
    Point3,
    Vector2,
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
        mesh::{
            Mesh,
            MeshBuilder,
            MeshPlugin,
        },
        texture_atlas::AtlasId,
    },
    voxel::{
        Voxel,
        block_face::BlockFace,
        greedy_quads::GreedyMesher,
    },
    wgpu::WgpuContext,
};

pub const CHUNK_SIDE_LENGTH_LOG2: u8 = 2;
pub const CHUNK_SIDE_LENGTH: u16 = 1 << CHUNK_SIDE_LENGTH_LOG2;
pub const CHUNK_NUM_VOXELS: usize = 1 << (3 * CHUNK_SIDE_LENGTH_LOG2);

pub struct FlatChunkPlugin<V> {
    _phantom: PhantomData<fn() -> V>,
}

impl<V> Default for FlatChunkPlugin<V> {
    fn default() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }
}

impl<V> Plugin for FlatChunkPlugin<V>
where
    //V: VoxelTexture + Send + Sync + 'static,
    V: Voxel,
{
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder
            .add_plugin(MeshPlugin)?
            .add_message::<MeshChunkRequest>()
            .add_systems(schedule::PostUpdate, mesh_chunks::<V>);
        Ok(())
    }
}

#[derive(Clone, Component)]
#[component(on_add = chunk_added, on_remove = chunk_removed)]
pub struct FlatChunk<V> {
    pub voxels: Box<[V; CHUNK_NUM_VOXELS]>,
}

impl<V> FlatChunk<V> {
    pub fn from_fn(mut f: impl FnMut(Point3<u16>) -> V) -> Self {
        let mut voxels = Box::new_uninit_slice(CHUNK_NUM_VOXELS);

        // fixme: memory leak when f panics
        for (i, voxel) in voxels.iter_mut().enumerate() {
            let point = Point3::from(morton_decode::<u16, 3>(i.try_into().unwrap()));
            voxel.write(f(point));
        }

        let voxels = unsafe { voxels.assume_init() };
        let voxels: Box<[V; CHUNK_NUM_VOXELS]> =
            voxels.try_into().unwrap_or_else(|_| unreachable!());

        Self { voxels }
    }
}

impl<V> FlatChunk<V> {
    pub fn naive_mesh(
        &self,
        mesh_builder: &mut MeshBuilder,
        mut texture: impl FnMut(&V) -> Option<AtlasId>,
    ) {
        for (i, voxel) in self.voxels.iter().enumerate() {
            if let Some(texture) = texture(voxel) {
                let point = Point3::from(morton_decode::<u16, 3>(i.try_into().unwrap()));

                for face in BlockFace::ALL {
                    let mut point = point;
                    match face {
                        BlockFace::Right => {
                            point.x += 1;
                        }
                        BlockFace::Up => {
                            point.y += 1;
                        }
                        BlockFace::Back => {
                            point.z += 1;
                        }
                        _ => {}
                    }

                    mesh_builder.push_block_face(point, Vector2::repeat(1), face, texture.into());
                }
            }
        }
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

fn mesh_chunks<V>(
    wgpu: Res<WgpuContext>,
    mut requests: MessageReader<MeshChunkRequest>,
    chunk_data: Populated<&FlatChunk<V>>,
    mut commands: Commands,
    mut mesh_builder: Local<MeshBuilder>,
    voxel_param: StaticSystemParam<V::Data>,
    mut greedy_mesher: Local<GreedyMesher<V>>,
) where
    V: Voxel,
{
    for request in requests.read() {
        tracing::debug!(entity = ?request.entity, "meshing chunk");

        if let Ok(chunk) = chunk_data.get(request.entity) {
            let mut entity = commands.entity(request.entity);

            let t_start = Instant::now();
            //chunk.naive_mesh(&mut mesh_builder, |voxel| voxel.texture(&voxel_param));
            greedy_mesher.mesh(&chunk.voxels, &mut mesh_builder, &voxel_param);
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
