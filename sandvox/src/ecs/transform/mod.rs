mod global;
mod local;
mod systems;

use bevy_ecs::schedule::{
    IntoScheduleConfigs,
    SystemSet,
};
use color_eyre::eyre::Error;

pub use crate::ecs::transform::{
    global::GlobalTransform,
    local::LocalTransform,
};
use crate::ecs::{
    plugin::{
        Plugin,
        WorldBuilder,
    },
    schedule::{
        PostStartup,
        PostUpdate,
    },
    transform::systems::{
        mark_dirty_trees,
        propagate_parent_transforms,
        sync_simple_transforms,
    },
};

/// Set enum for the systems relating to transform propagation
#[derive(Debug, Hash, PartialEq, Eq, Clone, SystemSet)]
pub enum TransformSystems {
    /// Propagates changes in transform to children's
    /// [`GlobalTransform`](crate::components::GlobalTransform)
    Propagate,
}

#[derive(Clone, Copy, Debug, Default)]
pub struct TransformHierarchyPlugin;

impl Plugin for TransformHierarchyPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        //context.add_system(schedule::PostUpdate, update_transform_hierarchy);

        builder
            // add transform systems to startup so the first update is "correct"
            .add_systems(
                PostStartup,
                (
                    mark_dirty_trees,
                    propagate_parent_transforms,
                    sync_simple_transforms,
                )
                    .chain()
                    .in_set(TransformSystems::Propagate),
            )
            .add_systems(
                PostUpdate,
                (
                    mark_dirty_trees,
                    propagate_parent_transforms,
                    // TODO: Adjust the internal parallel queries to make this system more
                    // efficiently share and fill CPU time.
                    sync_simple_transforms,
                )
                    .chain()
                    .in_set(TransformSystems::Propagate),
            );

        Ok(())
    }
}
