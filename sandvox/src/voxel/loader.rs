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
    resource::Resource,
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
    collide::Aabb,
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
    render::camera::FrustrumCulled,
    voxel::{
        chunk::ChunkShape,
        chunk_generator::GenerateChunk,
        chunk_map::{
            ChunkMap,
            ChunkPosition,
        },
    },
};

#[derive(Clone, Copy, Debug, Default)]
pub struct ChunkLoaderPlugin<S> {
    pub shape: S,
}

impl<S> Plugin for ChunkLoaderPlugin<S>
where
    S: ChunkShape,
{
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder
            .insert_resource(ChunkLoaderShape(self.shape.clone()))
            .add_systems(
                schedule::PostUpdate,
                (
                    create_chunk_loader_states::<S>,
                    update_chunk_loader_states::<S>,
                    remove_chunk_loader_states,
                )
                    .after(TransformSystems::Propagate),
            );

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Component)]
pub struct ChunkLoader {
    pub radius: Vector3<u32>,
}

#[derive(Clone, Copy, Debug, Component)]
struct ChunkLoaderState {
    chunk_position: Point3<i32>,
}

fn create_chunk_loader_states<S>(
    mut new_chunk_loaders: Query<
        (Entity, &ChunkLoader, &GlobalTransform),
        (
            Or<(Added<ChunkLoader>, Added<GlobalTransform>)>,
            Without<ChunkLoaderState>,
        ),
    >,
    mut commands: Commands,
    mut load_chunks: LoadChunks<S>,
) where
    S: ChunkShape,
{
    for (entity, chunk_loader, transform) in &mut new_chunk_loaders {
        let chunk_position = chunk_position_from_transform::<S>(&load_chunks.shape.0, transform);

        commands
            .entity(entity)
            .insert(ChunkLoaderState { chunk_position });

        tracing::debug!(?chunk_position, radius=?chunk_loader.radius, "trigger chunk loads");
        load_chunks.load_all(all_chunks_in_range(chunk_position, chunk_loader.radius));
    }
}

fn update_chunk_loader_states<S>(
    changed_chunk_loaders: Query<
        (&ChunkLoader, &mut ChunkLoaderState, &GlobalTransform),
        Or<(Changed<ChunkLoader>, Changed<GlobalTransform>)>,
    >,
    mut load_chunks: LoadChunks<S>,
) where
    S: ChunkShape,
{
    for (chunk_loader, mut state, transform) in changed_chunk_loaders {
        let chunk_position = chunk_position_from_transform::<S>(&load_chunks.shape.0, transform);
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

#[derive(Debug, Resource)]
struct ChunkLoaderShape<S>(S);

#[derive(SystemParam)]
struct LoadChunks<'w, 's, S>
where
    S: ChunkShape,
{
    chunk_map: Res<'w, ChunkMap>,
    commands: Commands<'w, 's>,
    shape: Res<'w, ChunkLoaderShape<S>>,
}

impl<'w, 's, S> LoadChunks<'w, 's, S>
where
    S: ChunkShape,
{
    fn load_all(&mut self, positions: impl IntoIterator<Item = Point3<i32>>) {
        for chunk_position in positions {
            if !self.chunk_map.contains(chunk_position) {
                // note: creating an entity with a ChunkPosition will cause this entity to be
                // inserted into the chunk map
                //
                // though on second thought it might be a good idea to make sure this can't
                // endlessly create entities if e.g. the chunk map system doesn't work.

                let chunk_size: i32 = self.shape.0.side_length().try_into().unwrap();
                let origin = (chunk_size as i32 * chunk_position).cast::<f32>();
                let aabb = Aabb::from_size(origin, Vector3::repeat(chunk_size as f32));

                let entity = self
                    .commands
                    .spawn((
                        ChunkPosition(chunk_position),
                        LocalTransform::from(origin),
                        GenerateChunk {
                            shape: self.shape.0.clone(),
                        },
                        FrustrumCulled { aabb },
                    ))
                    .id();

                tracing::trace!(?chunk_position, ?entity, "start loading chunk");
            }
        }
    }
}

fn chunk_position_from_transform<S>(shape: &S, transform: &GlobalTransform) -> Point3<i32>
where
    S: ChunkShape,
{
    let chunk_size: i32 = shape.side_length().try_into().unwrap();

    (transform
        .position()
        .coords
        .try_cast::<i32>()
        .unwrap()
        .map(|c| c.div_euclid(chunk_size)))
    .into()
}

fn all_chunks_in_range(
    position: Point3<i32>,
    radius: Vector3<u32>,
) -> impl Iterator<Item = Point3<i32>> {
    let radius = radius.cast::<i32>();

    (-radius.z..=radius.z).flat_map(move |z| {
        (-radius.y..=radius.y).flat_map(move |y| {
            (-radius.x..=radius.x).map(move |x| position + Vector3::new(x, y, z))
        })
    })
}

fn new_chunks_in_range(
    _old: Point3<i32>,
    new: Point3<i32>,
    radius: Vector3<u32>,
) -> impl Iterator<Item = Point3<i32>> {
    // todo: just return chunks that were not in range before
    all_chunks_in_range(new, radius)
}
