use std::{
    any::{
        Any,
        TypeId,
        type_name,
    },
    collections::HashSet,
};

use bevy_ecs::{
    message::{
        Message,
        MessageRegistry,
        Messages,
        message_update_system,
    },
    resource::Resource,
    schedule::{
        IntoScheduleConfigs,
        Schedule,
        ScheduleLabel,
        Schedules,
    },
    system::ScheduleSystem,
    world::World,
};
use color_eyre::eyre::Error;

use crate::ecs::schedule;

pub trait Plugin: Send + Sync + 'static {
    fn name(&self) -> &'static str {
        type_name::<Self>()
    }

    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error>;
}

#[derive(Debug)]
pub struct WorldBuilder {
    pub world: World,
    registered_plugins: HashSet<TypeId>,
}

impl Default for WorldBuilder {
    fn default() -> Self {
        let mut schedules = Schedules::new();

        schedules.insert(Schedule::new(schedule::Startup));
        schedules.insert(Schedule::new(schedule::PostStartup));
        schedules.insert(Schedule::new(schedule::PreUpdate));
        schedules.insert(Schedule::new(schedule::Update));
        schedules.insert(Schedule::new(schedule::PostStartup));
        schedules.insert(Schedule::new(schedule::Render));

        schedules.add_systems(schedule::PreUpdate, message_update_system);

        let mut world = World::new();
        world.insert_resource(schedules);

        Self {
            world,
            registered_plugins: HashSet::new(),
        }
    }
}

impl WorldBuilder {
    pub fn build(mut self) -> World {
        self.world.run_schedule(schedule::Startup);
        self.world.run_schedule(schedule::PostStartup);

        self.world
    }

    pub fn register_plugin(&mut self, plugin: impl Plugin) -> Result<(), Error> {
        if self.registered_plugins.insert(plugin.type_id()) {
            plugin.setup(self)?;
        }
        Ok(())
    }

    pub fn insert_resource(&mut self, resource: impl Resource) -> &mut Self {
        self.world.insert_resource(resource);
        self
    }

    pub fn add_systems<M>(
        &mut self,
        schedule: impl ScheduleLabel,
        systems: impl IntoScheduleConfigs<ScheduleSystem, M>,
    ) -> &mut Self {
        let mut schedules = self.world.resource_mut::<Schedules>();
        schedules.add_systems(schedule, systems);
        self
    }

    pub fn register_message<M>(&mut self) -> &mut Self
    where
        M: Message,
    {
        if !self.world.contains_resource::<Messages<M>>() {
            MessageRegistry::register_message::<M>(&mut self.world);
        }
        self
    }
}
