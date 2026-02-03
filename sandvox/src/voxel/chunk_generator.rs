use std::{
    marker::PhantomData,
    sync::Arc,
};

use bevy_ecs::{
    component::Component,
    entity::Entity,
    resource::Resource,
    schedule::{
        IntoScheduleConfigs,
        common_conditions::resource_exists,
    },
    system::{
        Commands,
        Query,
        Res,
    },
    world::{
        CommandQueue,
        World,
    },
};
use color_eyre::eyre::Error;
use nalgebra::Point3;

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
    },
    voxel::{
        Voxel,
        chunk::{
            Chunk,
            ChunkShape,
        },
        chunk_map::{
            ChunkPosition,
            ChunkStatistics,
        },
    },
};

#[derive(Clone, Debug)]
pub struct ChunkGeneratorPlugin<V, S, G> {
    task_config: BackgroundTaskConfig,
    _marker: PhantomData<fn() -> (V, S, G)>,
}

impl<V, S, G> ChunkGeneratorPlugin<V, S, G> {
    #[inline]
    pub fn new(task_config: BackgroundTaskConfig) -> Self {
        Self {
            task_config,
            _marker: PhantomData,
        }
    }
}

impl<V, S, G> Default for ChunkGeneratorPlugin<V, S, G> {
    #[inline]
    fn default() -> Self {
        Self::new(Default::default())
    }
}

impl<V, S, G> Plugin for ChunkGeneratorPlugin<V, S, G>
where
    V: Voxel,
    G: ChunkGenerator<V, S> + Resource,
    S: ChunkShape,
{
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder.configure_background_task_queue::<GenerateChunkTask<V, S, G>>(self.task_config);

        builder.add_systems(
            schedule::Update,
            (
                make_chunk_generator_shared::<V, S, G>.run_if(resource_exists::<G>),
                dispatch_chunk_generation::<V, S, G>
                    .run_if(resource_exists::<SharedChunkGenerator<G>>),
            )
                .chain(),
        );

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Component)]
pub struct GenerateChunk<S> {
    pub shape: S,
}

#[derive(Clone, Debug, Resource)]
struct SharedChunkGenerator<G>(Arc<G>);

fn make_chunk_generator_shared<V, S, G>(world: &mut World)
where
    V: Voxel,
    S: ChunkShape,
    G: ChunkGenerator<V, S> + Resource,
{
    let chunk_generator = world.remove_resource::<G>().unwrap();
    world.insert_resource(SharedChunkGenerator(Arc::new(chunk_generator)));
}

fn dispatch_chunk_generation<V, S, G>(
    background_tasks: Res<BackgroundTaskPool>,
    chunk_generator: Res<SharedChunkGenerator<G>>,
    chunks: Query<(Entity, &ChunkPosition, &GenerateChunk<S>)>,
    mut commands: Commands,
) where
    V: Voxel,
    S: ChunkShape,
    G: ChunkGenerator<V, S>,
{
    background_tasks.push_tasks(
        chunks
            .iter()
            .filter(|(_entity, position, generate_chunk)| {
                !chunk_generator
                    .0
                    .early_discard(position.0, &generate_chunk.shape)
            })
            .map(|(entity, position, generate_chunk)| {
                commands.entity(entity).remove::<GenerateChunk<S>>();
                GenerateChunkTask::<V, S, G> {
                    position: position.0,
                    shape: generate_chunk.shape.clone(),
                    entity,
                    chunk_generator: chunk_generator.0.clone(),
                    _phantom: PhantomData,
                }
            }),
    );
}

#[derive(Debug)]
struct GenerateChunkTask<V, S, G> {
    position: Point3<i32>,
    shape: S,
    entity: Entity,
    chunk_generator: Arc<G>,
    _phantom: PhantomData<fn() -> V>,
}

impl<V, S, G> Task for GenerateChunkTask<V, S, G>
where
    V: Voxel,
    S: ChunkShape,
    G: ChunkGenerator<V, S>,
{
    fn run(self, world_modifications: &mut CommandQueue) {
        if let Some(chunk) = self
            .chunk_generator
            .generate_chunk(self.position, self.shape)
        {
            world_modifications.push(move |world: &mut World| {
                let mut chunk_statistics = world.resource_mut::<ChunkStatistics>();
                chunk_statistics.num_chunks_loaded += 1;
                chunk_statistics.bytes_chunks_loaded += chunk.byte_size();

                world.commands().entity(self.entity).insert(chunk);
            });
        }
    }
}

pub trait ChunkGenerator<V, S>: Send + Sync + 'static
where
    V: Voxel,
    S: ChunkShape,
{
    #[inline]
    fn early_discard(&self, position: Point3<i32>, shape: &S) -> bool {
        let _ = (position, shape);
        false
    }

    fn generate_chunk(&self, position: Point3<i32>, shape: S) -> Option<Chunk<V, S>>;
}
