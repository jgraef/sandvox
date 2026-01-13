use std::{
    any::{
        Any,
        TypeId,
    },
    collections::{
        HashMap,
        VecDeque,
        hash_map,
    },
    fmt::Debug,
    num::NonZero,
    sync::Arc,
};

use bevy_ecs::{
    resource::Resource,
    system::{
        Commands,
        Res,
    },
    world::CommandQueue,
};
use color_eyre::eyre::Error;
use parking_lot::{
    Condvar,
    Mutex,
};

use crate::ecs::{
    plugin::{
        Plugin,
        WorldBuilder,
    },
    schedule,
};

#[derive(Clone, Copy, Debug, Default)]
pub struct BackgroundTaskPlugin {
    pub num_threads: Option<NonZero<usize>>,
}

impl BackgroundTaskPlugin {
    pub fn max_threads() -> Self {
        Self::default()
    }

    pub fn with_num_threads(num_threads: NonZero<usize>) -> Self {
        Self {
            num_threads: Some(num_threads),
        }
    }
}

impl Plugin for BackgroundTaskPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        let num_threads = self
            .num_threads
            .or_else(|| std::thread::available_parallelism().ok())
            .unwrap_or(const { NonZero::new(1).unwrap() });

        let shared = Arc::new(Shared {
            condition: Condvar::new(),
            state: Mutex::new(State {
                active: true,
                task_queues: HashMap::new(),
                world_modifications: CommandQueue::default(),
                num_threads,
            }),
        });

        tracing::info!(num_threads, "Initializing background task pool");

        for i in 0..num_threads.get() {
            let shared = shared.clone();
            let _join_handle = std::thread::Builder::new()
                .name(format!("background-{i}"))
                .spawn(move || {
                    worker_thread(i, shared);
                });
        }

        builder
            .insert_resource(BackgroundTaskPool { shared })
            .add_systems(schedule::PostUpdate, apply_background_modifications);

        Ok(())
    }
}

fn apply_background_modifications(pool: Res<BackgroundTaskPool>, mut commands: Commands) {
    let mut state = pool.shared.state.lock();
    commands.append(&mut state.world_modifications);
}

pub trait WorldBuilderBackgroundTaskExt {
    fn configure_background_task_queue<T>(
        &self,
        queue_size: Option<NonZero<usize>>,
        num_threads: Option<NonZero<usize>>,
    ) where
        T: Task;
}

impl WorldBuilderBackgroundTaskExt for WorldBuilder {
    fn configure_background_task_queue<T>(
        &self,
        queue_size: Option<NonZero<usize>>,
        num_threads: Option<NonZero<usize>>,
    ) where
        T: Task,
    {
        let pool = self
            .world
            .get_resource::<BackgroundTaskPool>()
            .expect("BackgroundTaskPool not found. Have you added the BackgroundTaskPlugin?");

        pool.configure_queue::<T>(queue_size, num_threads);
    }
}

#[derive(Clone, Debug, Resource)]
pub struct BackgroundTaskPool {
    shared: Arc<Shared>,
}

impl BackgroundTaskPool {
    pub fn configure_queue<T>(
        &self,
        queue_size: Option<NonZero<usize>>,
        num_threads: Option<NonZero<usize>>,
    ) where
        T: Task,
    {
        let mut state = self.shared.state.lock();
        let num_threads = num_threads.map_or(state.num_threads, |num_threads| {
            state.num_threads.min(num_threads)
        });
        let queue_size = queue_size.unwrap_or_else(|| default_queue_size(num_threads));

        match state.task_queues.entry(TypeId::of::<T>()) {
            hash_map::Entry::Occupied(mut occupied_entry) => {
                let task_queue = occupied_entry.get_mut();
                task_queue.num_threads = num_threads;
                task_queue.queue_size = queue_size;
            }
            hash_map::Entry::Vacant(vacant_entry) => {
                vacant_entry.insert(TaskQueue::new::<T>(queue_size, num_threads));
            }
        }
    }

    pub fn push_tasks<T>(&self, tasks: impl IntoIterator<Item = T>)
    where
        T: Task,
    {
        let mut state = self.shared.state.lock();
        let num_threads = state.num_threads;

        let task_queue = state
            .task_queues
            .entry(TypeId::of::<T>())
            .or_insert_with(move || {
                TaskQueue::new::<T>(default_queue_size(num_threads), num_threads)
            });

        let num_free = task_queue.queue_size.get() - task_queue.num_queued;

        if num_free > 0 {
            let inner = (&mut *task_queue.inner as &mut dyn Any)
                .downcast_mut::<TaskQueueInner<T>>()
                .unwrap();

            let num_queued = tasks
                .into_iter()
                .take(num_free)
                .map(|task| {
                    inner.queue.push_back(task);
                })
                .count();

            if num_queued > 0 {
                task_queue.num_queued += num_queued;
                self.shared.condition.notify_all();
            }
        }
    }
}

impl Drop for BackgroundTaskPool {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock();
        state.active = false;
        self.shared.condition.notify_all();
    }
}

pub trait Task: Send + Sync + 'static {
    fn run(self, world_modifications: &mut CommandQueue);
}

#[derive(Debug)]
struct Shared {
    condition: Condvar,
    state: Mutex<State>,
}

#[derive(Debug)]
struct State {
    active: bool,
    task_queues: HashMap<TypeId, TaskQueue>,
    world_modifications: CommandQueue,
    num_threads: NonZero<usize>,
}

#[derive(derive_more::Debug)]
struct TaskQueue {
    queue_size: NonZero<usize>,
    num_threads: NonZero<usize>,
    num_queued: usize,
    num_active: usize,
    #[debug(skip)]
    inner: Box<dyn DynTaskQueueInner>,
}

impl TaskQueue {
    fn new<T>(queue_size: NonZero<usize>, num_threads: NonZero<usize>) -> Self
    where
        T: Task,
    {
        Self {
            queue_size,
            num_threads,
            num_queued: 0,
            num_active: 0,
            inner: Box::new(TaskQueueInner::<T> {
                queue: VecDeque::with_capacity(queue_size.get()),
            }),
        }
    }
}

const fn default_queue_size(num_threads: NonZero<usize>) -> NonZero<usize> {
    NonZero::new(num_threads.get() * 2).unwrap()
}

trait DynTaskQueueInner: Send + Sync + Any + 'static {
    fn pop(&mut self) -> Box<dyn FnOnce(&mut CommandQueue)>;
}

struct TaskQueueInner<T>
where
    T: Task,
{
    queue: VecDeque<T>,
}

impl<T> DynTaskQueueInner for TaskQueueInner<T>
where
    T: Task,
{
    fn pop(&mut self) -> Box<dyn FnOnce(&mut CommandQueue)> {
        let task = self.queue.pop_front().unwrap();
        Box::new(move |world_modifications| task.run(world_modifications))
    }
}

fn worker_thread(thread_id: usize, shared: Arc<Shared>) {
    let span = tracing::info_span!("worker thread", thread_id);
    let _guard = span.enter();

    let mut world_modifications = CommandQueue::default();
    let mut current_task = None;

    loop {
        let task = 'get_task: {
            let mut state = shared.state.lock();

            // move any pending world modifications from last loop iteration into shared
            // state.
            state.world_modifications.append(&mut world_modifications);

            if let Some(task_id) = current_task.take() {
                let task_queue = state.task_queues.get_mut(&task_id).unwrap();
                task_queue.num_active -= 1;
            }

            loop {
                // check if task pool is still active
                if !state.active {
                    return;
                }

                // note: instead of a linear scan we could keep a hashset in state that tells us
                // which queues have items, but the number of queues is expected to be very low,
                // so this might be faster.
                for (task_id, task_queue) in &mut state.task_queues {
                    if task_queue.num_queued > 0
                        && task_queue.num_active < task_queue.num_threads.get()
                    {
                        task_queue.num_queued -= 1;
                        task_queue.num_active += 1;
                        current_task = Some(*task_id);
                        break 'get_task task_queue.inner.pop();
                    }
                }

                // didn't find any task, block until something gets pushed
                shared.condition.wait(&mut state);
            }
        };

        // run task
        task(&mut world_modifications);
    }
}
