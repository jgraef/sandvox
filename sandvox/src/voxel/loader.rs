use bevy_ecs::{
    component::Component,
    entity::Entity,
    query::{
        Added,
        Changed,
        Or,
        With,
        Without,
    },
    schedule::IntoScheduleConfigs,
    system::{
        Commands,
        Query,
        Res,
        SystemParam,
    },
};
use color_eyre::eyre::Error;
use nalgebra::{
    Point3,
    Vector3,
};

use crate::{
    ecs::{
        plugin::{
            Plugin,
            WorldBuilder,
        },
        schedule,
        transform::{
            GlobalTransform,
            LocalTransform,
            TransformSystems,
        },
    },
    voxel::{
        chunk_generator::GenerateChunk,
        chunk_map::{
            ChunkMap,
            ChunkPosition,
        },
    },
};

#[derive(Clone, Copy, Debug, Default)]
pub struct ChunkLoaderPlugin<const CHUNK_SIZE: usize>;

impl<const CHUNK_SIZE: usize> Plugin for ChunkLoaderPlugin<CHUNK_SIZE> {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder.add_systems(
            schedule::PostUpdate,
            (
                create_chunk_loader_states::<CHUNK_SIZE>,
                update_chunk_loader_states::<CHUNK_SIZE>,
                remove_chunk_loader_states,
            )
                .after(TransformSystems::Propagate),
        );

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Component)]
pub struct ChunkLoader {
    pub radius: u32,
}

#[derive(Clone, Copy, Debug, Component)]
struct ChunkLoaderState {
    chunk_position: Point3<i32>,
}

fn create_chunk_loader_states<const CHUNK_SIZE: usize>(
    mut new_chunk_loaders: Query<
        (Entity, &ChunkLoader, &GlobalTransform),
        (
            Or<(Added<ChunkLoader>, Added<GlobalTransform>)>,
            Without<ChunkLoaderState>,
        ),
    >,
    mut commands: Commands,
    mut load_chunks: LoadChunks<CHUNK_SIZE>,
) {
    for (entity, chunk_loader, transform) in &mut new_chunk_loaders {
        let chunk_position = chunk_position_from_transform::<CHUNK_SIZE>(transform);

        commands
            .entity(entity)
            .insert(ChunkLoaderState { chunk_position });

        tracing::debug!(?chunk_position, radius=?chunk_loader.radius, "trigger chunk loads");
        load_chunks.load_all(all_chunks_in_range(chunk_position, chunk_loader.radius));
    }
}

fn update_chunk_loader_states<const CHUNK_SIZE: usize>(
    changed_chunk_loaders: Query<
        (&ChunkLoader, &mut ChunkLoaderState, &GlobalTransform),
        Or<(Changed<ChunkLoader>, Changed<GlobalTransform>)>,
    >,
    mut load_chunks: LoadChunks<CHUNK_SIZE>,
) {
    for (chunk_loader, mut state, transform) in changed_chunk_loaders {
        let chunk_position = chunk_position_from_transform::<CHUNK_SIZE>(transform);
        if chunk_position != state.chunk_position {
            tracing::debug!(?chunk_position, radius=?chunk_loader.radius, "trigger chunk loads");

            load_chunks.load_all(new_chunks_in_range(
                state.chunk_position,
                chunk_position,
                chunk_loader.radius,
            ));

            // todo: possibly remove chunk generation requests from chunks that are not in
            // range anymore

            state.chunk_position = chunk_position;
        }
    }
}

fn remove_chunk_loader_states(
    removed_chunk_loaders: Query<
        Entity,
        (
            With<ChunkLoaderState>,
            Or<(Without<ChunkLoader>, Without<GlobalTransform>)>,
        ),
    >,
    mut commands: Commands,
) {
    for entity in removed_chunk_loaders {
        commands.entity(entity).remove::<ChunkLoaderState>();
    }
}

#[derive(SystemParam)]
struct LoadChunks<'w, 's, const CHUNK_SIZE: usize> {
    chunk_map: Res<'w, ChunkMap>,
    commands: Commands<'w, 's>,
}

impl<'w, 's, const CHUNK_SIZE: usize> LoadChunks<'w, 's, CHUNK_SIZE> {
    fn load_all(&mut self, positions: impl IntoIterator<Item = Point3<i32>>) {
        for position in positions {
            if !self.chunk_map.contains(position) {
                // note: creating an entity with a ChunkPosition will cause this entity to be
                // inserted into the chunk map

                let transform: LocalTransform = (CHUNK_SIZE as i32 * position).cast::<f32>().into();

                let entity = self
                    .commands
                    .spawn((ChunkPosition(position), transform, GenerateChunk))
                    .id();

                tracing::trace!(?position, ?entity, "start loading chunk");
            }
        }
    }
}

fn chunk_position_from_transform<const CHUNK_SIZE: usize>(
    transform: &GlobalTransform,
) -> Point3<i32> {
    (transform
        .position()
        .coords
        .try_cast::<i32>()
        .unwrap()
        .map(|c| c.div_euclid(CHUNK_SIZE as i32)))
    .into()
}

fn all_chunks_in_range(position: Point3<i32>, radius: u32) -> impl Iterator<Item = Point3<i32>> {
    let radius = radius as i32;

    (-radius..=radius).flat_map(move |z| {
        (-radius..=radius)
            .flat_map(move |y| (-radius..=radius).map(move |x| position + Vector3::new(x, y, z)))
    })
}

fn new_chunks_in_range(
    _old: Point3<i32>,
    new: Point3<i32>,
    radius: u32,
) -> impl Iterator<Item = Point3<i32>> {
    // todo: just return chunks that were not in range before
    all_chunks_in_range(new, radius)
}
