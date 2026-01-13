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
                task_queues: vec![],
                task_queues_by_type_id: HashMap::new(),
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

#[derive(Clone, Copy, Debug, Default)]
pub struct BackgroundTaskConfig {
    pub queue_size: Option<NonZero<usize>>,
    pub num_threads: Option<NonZero<usize>>,
}

pub trait WorldBuilderBackgroundTaskExt {
    fn configure_background_task_queue<T>(&self, config: BackgroundTaskConfig)
    where
        T: Task;
}

impl WorldBuilderBackgroundTaskExt for WorldBuilder {
    fn configure_background_task_queue<T>(&self, config: BackgroundTaskConfig)
    where
        T: Task,
    {
        let pool = self
            .world
            .get_resource::<BackgroundTaskPool>()
            .expect("BackgroundTaskPool not found. Have you added the BackgroundTaskPlugin?");

        pool.configure_queue::<T>(config);
    }
}

#[derive(Clone, Debug, Resource)]
pub struct BackgroundTaskPool {
    shared: Arc<Shared>,
}

impl BackgroundTaskPool {
    pub fn configure_queue<T>(&self, config: BackgroundTaskConfig)
    where
        T: Task,
    {
        let mut state = self.shared.state.lock();
        let state = &mut *state;

        let num_threads = config.num_threads.map_or(state.num_threads, |num_threads| {
            state.num_threads.min(num_threads)
        });

        let queue_size = config
            .queue_size
            .unwrap_or_else(|| default_queue_size(num_threads));

        match state.task_queues_by_type_id.entry(TypeId::of::<T>()) {
            hash_map::Entry::Occupied(occupied_entry) => {
                let task_queue = &mut state.task_queues[*occupied_entry.get()];
                task_queue.num_threads = num_threads;
                task_queue.queue_size = queue_size;
            }
            hash_map::Entry::Vacant(vacant_entry) => {
                let index = state.task_queues.len();
                state
                    .task_queues
                    .push(TaskQueue::new::<T>(queue_size, num_threads));
                vacant_entry.insert(index);
            }
        }
    }

    pub fn push_tasks<T>(&self, tasks: impl IntoIterator<Item = T>)
    where
        T: Task,
    {
        let mut state = self.shared.state.lock();
        let state = &mut *state;

        let task_queue = {
            let index = state
                .task_queues_by_type_id
                .entry(TypeId::of::<T>())
                .or_insert_with(|| {
                    let index = state.task_queues.len();
                    state.task_queues.push(TaskQueue::new::<T>(
                        default_queue_size(state.num_threads),
                        state.num_threads,
                    ));
                    index
                });
            &mut state.task_queues[*index]
        };

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
    task_queues: Vec<TaskQueue>,
    task_queues_by_type_id: HashMap<TypeId, usize>,
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
    let mut active_task: Option<usize> = None;

    loop {
        let task = 'get_task: {
            let mut state = shared.state.lock();

            // move any pending world modifications from last loop iteration into shared
            // state.
            state.world_modifications.append(&mut world_modifications);

            // if we just processed a task, make sure to decrement the active counter.
            // this also returns from which position in the task_queues array we'll scan for
            // the next item
            let cursor = if let Some(task_id) = active_task.take() {
                let task_queue = &mut state.task_queues[task_id];
                task_queue.num_active -= 1;

                // scan for next item starting from the next queue
                let num_task_queues = state.task_queues.len();
                (task_id + 1) % num_task_queues
            }
            else {
                0
            };

            loop {
                // check if task pool is still active
                if !state.active {
                    return;
                }

                // note: instead of a linear scan we could keep a hashset in state that tells us
                // which queues have items, but the number of queues is expected to be very low,
                // so this might be faster.
                let num_task_queues = state.task_queues.len();
                for task_id in (cursor..num_task_queues).into_iter().chain(0..cursor) {
                    let task_queue = &mut state.task_queues[task_id];

                    if task_queue.num_queued > 0
                        && task_queue.num_active < task_queue.num_threads.get()
                    {
                        task_queue.num_queued -= 1;
                        task_queue.num_active += 1;
                        active_task = Some(task_id);
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
