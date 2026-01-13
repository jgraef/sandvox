use std::{
    marker::PhantomData,
    sync::Arc,
};

use bevy_ecs::{
    component::Component,
    entity::Entity,
    query::With,
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
        chunk::Chunk,
        chunk_map::ChunkPosition,
    },
};

#[derive(Clone, Debug)]
pub struct ChunkGeneratorPlugin<V, G, const CHUNK_SIZE: usize> {
    task_config: BackgroundTaskConfig,
    _phantom: PhantomData<(V, G)>,
}

impl<V, G, const CHUNK_SIZE: usize> ChunkGeneratorPlugin<V, G, CHUNK_SIZE> {
    pub fn new(task_config: BackgroundTaskConfig) -> Self {
        Self {
            task_config,
            _phantom: PhantomData,
        }
    }
}

impl<V, G, const CHUNK_SIZE: usize> Default for ChunkGeneratorPlugin<V, G, CHUNK_SIZE> {
    fn default() -> Self {
        Self::new(Default::default())
    }
}

impl<V, G, const CHUNK_SIZE: usize> Plugin for ChunkGeneratorPlugin<V, G, CHUNK_SIZE>
where
    V: Voxel,
    G: ChunkGenerator<V, CHUNK_SIZE>,
{
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder.configure_background_task_queue::<GenerateChunkTask<V, G, CHUNK_SIZE>>(
            self.task_config,
        );

        builder.add_systems(
            schedule::Update,
            (
                make_chunk_generator_shared::<V, G, CHUNK_SIZE>.run_if(resource_exists::<G>),
                dispatch_chunk_generation::<V, G, CHUNK_SIZE>
                    .run_if(resource_exists::<SharedChunkGenerator<G>>),
            )
                .chain(),
        );

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default, Component)]
pub struct GenerateChunk;

#[derive(Clone, Debug, Resource)]
struct SharedChunkGenerator<G>(Arc<G>);

fn make_chunk_generator_shared<V, G, const CHUNK_SIZE: usize>(world: &mut World)
where
    V: Voxel,
    G: ChunkGenerator<V, CHUNK_SIZE>,
{
    let chunk_generator = world.remove_resource::<G>().unwrap();
    world.insert_resource(SharedChunkGenerator(Arc::new(chunk_generator)));
}

fn dispatch_chunk_generation<V, G, const CHUNK_SIZE: usize>(
    background_tasks: Res<BackgroundTaskPool>,
    chunk_generator: Res<SharedChunkGenerator<G>>,
    chunks: Query<(Entity, &ChunkPosition), With<GenerateChunk>>,
    mut commands: Commands,
) where
    V: Voxel,
    G: ChunkGenerator<V, CHUNK_SIZE>,
{
    background_tasks.push_tasks(
        chunks
            .iter()
            .filter(|(_entity, position)| chunk_generator.0.filter(position.0))
            .map(|(entity, position)| {
                commands.entity(entity).remove::<GenerateChunk>();
                GenerateChunkTask::<V, G, CHUNK_SIZE> {
                    position: position.0,
                    entity,
                    chunk_generator: chunk_generator.0.clone(),
                    _phantom: PhantomData,
                }
            }),
    );
}

#[derive(Debug)]
struct GenerateChunkTask<V, G, const CHUNK_SIZE: usize> {
    position: Point3<i32>,
    entity: Entity,
    chunk_generator: Arc<G>,
    _phantom: PhantomData<V>,
}

impl<V, G, const CHUNK_SIZE: usize> Task for GenerateChunkTask<V, G, CHUNK_SIZE>
where
    V: Voxel,
    G: ChunkGenerator<V, CHUNK_SIZE>,
{
    fn run(self, world_modifications: &mut CommandQueue) {
        if let Some(chunk) = self.chunk_generator.generate_chunk(self.position) {
            world_modifications.push(move |world: &mut World| {
                world.commands().entity(self.entity).insert(chunk);
            });
        }
    }
}

pub trait ChunkGenerator<V, const CHUNK_SIZE: usize>: Resource + Send + Sync + 'static {
    fn filter(&self, position: Point3<i32>) -> bool {
        let _ = position;
        true
    }

    fn generate_chunk(&self, chunk_position: Point3<i32>) -> Option<Chunk<V, CHUNK_SIZE>>;
}
