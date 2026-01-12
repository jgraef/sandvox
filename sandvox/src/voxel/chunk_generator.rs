use std::{
    collections::VecDeque,
    marker::PhantomData,
    num::NonZero,
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
        In,
        Query,
        Res,
    },
};
use color_eyre::eyre::Error;
use nalgebra::Point3;
use parking_lot::{
    Condvar,
    Mutex,
};

use crate::{
    ecs::{
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
    config: ChunkGeneratorConfig,
    _phantom: PhantomData<(V, G)>,
}

impl<V, G, const CHUNK_SIZE: usize> Default for ChunkGeneratorPlugin<V, G, CHUNK_SIZE> {
    fn default() -> Self {
        Self {
            config: Default::default(),
            _phantom: PhantomData,
        }
    }
}

impl<V, G, const CHUNK_SIZE: usize> ChunkGeneratorPlugin<V, G, CHUNK_SIZE> {
    pub fn new(config: ChunkGeneratorConfig) -> Self {
        Self {
            config,
            _phantom: PhantomData,
        }
    }
}

impl<V, G, const CHUNK_SIZE: usize> Plugin for ChunkGeneratorPlugin<V, G, CHUNK_SIZE>
where
    V: Voxel,
    G: ChunkGenerator<V, CHUNK_SIZE>,
{
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder.insert_resource(self.config.clone()).add_systems(
            schedule::PostUpdate,
            queue_chunk_generation_requests::<V, G, CHUNK_SIZE>
                .run_if(resource_exists::<Proxy<V, G, CHUNK_SIZE>>),
        );

        Ok(())
    }
}

#[derive(Clone, Debug, Resource)]
pub struct ChunkGeneratorConfig {
    pub queue_size: NonZero<usize>,
}

impl Default for ChunkGeneratorConfig {
    fn default() -> Self {
        Self {
            queue_size: const { NonZero::new(64).unwrap() },
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Component)]
pub struct GenerateChunk;

pub fn spawn_chunk_generator_thread<V, G, const CHUNK_SIZE: usize>(
    In(chunk_generator): In<G>,
    config: Res<ChunkGeneratorConfig>,
    mut commands: Commands,
) where
    V: Voxel,
    G: ChunkGenerator<V, CHUNK_SIZE>,
{
    let shared = Arc::new(Shared {
        condition: Condvar::new(),
        state: Mutex::new(State::<V, CHUNK_SIZE> {
            active: true,
            request_queue: VecDeque::with_capacity(config.queue_size.get()),
            generated_chunks: Vec::with_capacity(config.queue_size.get()),
            queue_size: config.queue_size,
        }),
        chunk_generator,
    });

    let _join_handle = {
        let shared = shared.clone();

        std::thread::spawn(move || {
            let worldgen_thread = WorldGenThread::new(shared);
            worldgen_thread.run()
        })
    };

    commands.insert_resource(Proxy { shared })
}

fn queue_chunk_generation_requests<V, G, const CHUNK_SIZE: usize>(
    proxy: Res<Proxy<V, G, CHUNK_SIZE>>,
    chunks: Query<(Entity, &ChunkPosition), With<GenerateChunk>>,
    mut commands: Commands,
) where
    V: Voxel,
    G: ChunkGenerator<V, CHUNK_SIZE>,
{
    let mut state = proxy.shared.state.lock();

    // read back all generated chunks and attach them to the entities
    for chunk in state.generated_chunks.drain(..) {
        commands.entity(chunk.entity).insert(chunk.chunk);
    }

    // enqueue as many chunk generation requests as possible
    let mut chunks = chunks.iter();

    while state.request_queue.len() < state.queue_size.get()
        && let Some((entity, position)) = chunks.next()
    {
        if proxy.shared.chunk_generator.filter(position.0) {
            state.request_queue.push_back(Request {
                position: position.0,
                entity,
            });
            commands.entity(entity).remove::<GenerateChunk>();
        }
    }

    proxy.shared.condition.notify_all();
}

#[derive(Debug, Resource)]
struct Proxy<V, G, const CHUNK_SIZE: usize> {
    shared: Arc<Shared<V, G, CHUNK_SIZE>>,
}

impl<V, G, const CHUNK_SIZE: usize> Drop for Proxy<V, G, CHUNK_SIZE> {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock();
        state.active = false;
        self.shared.condition.notify_all();
    }
}

#[derive(Debug)]
struct Shared<V, G, const CHUNK_SIZE: usize> {
    condition: Condvar,
    state: Mutex<State<V, CHUNK_SIZE>>,
    chunk_generator: G,
}

#[derive(Debug)]
struct State<V, const CHUNK_SIZE: usize> {
    active: bool,
    request_queue: VecDeque<Request>,
    generated_chunks: Vec<GeneratedChunk<V, CHUNK_SIZE>>,
    queue_size: NonZero<usize>,
}

#[derive(Debug)]
struct Request {
    position: Point3<i32>,
    entity: Entity,
}

#[derive(Debug)]
struct GeneratedChunk<V, const CHUNK_SIZE: usize> {
    entity: Entity,
    chunk: Chunk<V, CHUNK_SIZE>,
}

#[derive(Debug)]
struct WorldGenThread<V, G, const CHUNK_SIZE: usize>
where
    G: ChunkGenerator<V, CHUNK_SIZE>,
{
    shared: Arc<Shared<V, G, CHUNK_SIZE>>,
    workspace: G::Workspace,
}

impl<V, G, const CHUNK_SIZE: usize> WorldGenThread<V, G, CHUNK_SIZE>
where
    G: ChunkGenerator<V, CHUNK_SIZE>,
{
    fn new(shared: Arc<Shared<V, G, CHUNK_SIZE>>) -> Self {
        let workspace = shared.chunk_generator.create_workspace();

        Self { shared, workspace }
    }

    fn run(mut self) {
        let mut generated_chunk = None;

        loop {
            // this is the critical section where we lock the shared state.
            //
            // we'll only put in any generated chunk and pop a single request
            let request = {
                let mut state = self.shared.state.lock();

                // check if we're still supposed to run
                if !state.active {
                    return;
                }

                // send back any generated chunk
                state.generated_chunks.extend(generated_chunk.take());

                // get the next request
                loop {
                    if let Some(request) = state.request_queue.pop_front() {
                        break request;
                    }
                    else {
                        // queue empty, block until we get woken up
                        self.shared.condition.wait(&mut state);

                        // first make sure this thing is still going
                        if !state.active {
                            return;
                        }
                    }
                }
            };

            // generate chunk.
            //
            // the generated chunk (if any) is stored and put into the shared state when we
            // need to lock it for reading the next request in the next iteration of the
            // loop.
            generated_chunk = self
                .shared
                .chunk_generator
                .generate_chunk(&mut self.workspace, request.position)
                .map(|chunk| {
                    GeneratedChunk {
                        entity: request.entity,
                        chunk,
                    }
                });
        }
    }
}

impl<V, G, const CHUNK_SIZE: usize> Drop for WorldGenThread<V, G, CHUNK_SIZE>
where
    G: ChunkGenerator<V, CHUNK_SIZE>,
{
    fn drop(&mut self) {
        let mut state = self.shared.state.lock();
        state.active = false;
    }
}

pub trait ChunkGenerator<V, const CHUNK_SIZE: usize>: Send + Sync + 'static {
    type Workspace;

    fn create_workspace(&self) -> Self::Workspace;

    fn filter(&self, position: Point3<i32>) -> bool {
        let _ = position;
        true
    }

    fn generate_chunk(
        &self,
        workspace: &mut Self::Workspace,
        chunk_position: Point3<i32>,
    ) -> Option<Chunk<V, CHUNK_SIZE>>;
}
