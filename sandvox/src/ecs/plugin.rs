use std::{
    any::{
        Any,
        TypeId,
        type_name,
    },
    collections::{
        HashMap,
        HashSet,
    },
    fs::File,
    io::{
        BufWriter,
        Write,
    },
    path::Path,
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
        InternedSystemSet,
        IntoScheduleConfigs,
        NodeId,
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
        schedules.insert(Schedule::new(schedule::PostUpdate));

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
    pub fn write_schedule_graphs_to_dot(&self, path: impl AsRef<Path>) -> Result<(), Error> {
        tracing::debug!(path = %path.as_ref().display(), "writing schedule graphs to file");

        let writer = BufWriter::new(File::create(path)?);
        let schedules = self.world.resource::<Schedules>();
        write_schedule_graphs_to_dot(schedules, writer)?;
        Ok(())
    }

    pub fn build(&mut self) -> World {
        self.world.run_schedule(schedule::Startup);
        self.world.run_schedule(schedule::PostStartup);

        std::mem::take(&mut self.world)
    }

    pub fn add_plugin(&mut self, plugin: impl Plugin) -> Result<&mut Self, Error> {
        if self.registered_plugins.insert(plugin.type_id()) {
            plugin.setup(self)?;
        }
        Ok(self)
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

    pub fn configure_system_sets<M>(
        &mut self,
        schedule: impl ScheduleLabel,
        systems: impl IntoScheduleConfigs<InternedSystemSet, M>,
    ) -> &mut Self {
        let mut schedules = self.world.resource_mut::<Schedules>();
        schedules.configure_sets(schedule, systems);
        self
    }

    pub fn add_message<M>(&mut self) -> &mut Self
    where
        M: Message,
    {
        if !self.world.contains_resource::<Messages<M>>() {
            MessageRegistry::register_message::<M>(&mut self.world);
        }
        self
    }
}

fn write_schedule_graphs_to_dot<W>(schedules: &Schedules, mut writer: W) -> Result<(), Error>
where
    W: Write,
{
    let mut node_id = 0;

    writeln!(&mut writer, "digraph \"schedules\" {{")?;
    writeln!(&mut writer, "  rankdir=LR;")?;

    for (schedule_id, (_label, schedule)) in schedules.iter().enumerate() {
        let schedule_graph = schedule.graph();
        let mut nodes = HashMap::new();

        writeln!(&mut writer, "  subgraph \"cluster_{schedule_id}\" {{",)?;
        writeln!(&mut writer, "    newrank=true;")?;
        writeln!(&mut writer, "    rankdir=TB;")?;
        writeln!(&mut writer, "    label=\"{:?}\";", schedule.label())?;
        writeln!(&mut writer, "")?;

        for (system_key, system, _) in schedule_graph.systems.iter() {
            writeln!(
                &mut writer,
                "    n{node_id} [label=\"{}\"];",
                system.name().shortname()
            )?;
            nodes.insert(NodeId::System(system_key), node_id);
            node_id += 1;
        }
        writeln!(&mut writer, "")?;

        /*for (system_set_key, _system_set, _) in schedule_graph.system_sets.iter() {
            writeln!(&mut writer, "    n{node_id} [shape=box];",)?;
            nodes.insert(NodeId::Set(system_set_key), node_id);
            node_id += 1;
        }
        writeln!(&mut writer, "")?;*/

        for edge in schedule_graph.dependency().graph().all_edges() {
            if let Some((start, end)) = nodes.get(&edge.0).zip(nodes.get(&edge.1)) {
                writeln!(&mut writer, "    n{start} -> n{end};")?;
            }
        }
        writeln!(&mut writer, "")?;

        /*for edge in schedule_graph.hierarchy().graph().all_edges() {
            let start = nodes[&edge.0];
            let end = nodes[&edge.1];
            writeln!(&mut writer, "    n{start} -> n{end} [style=dotted];")?;
        }*/

        writeln!(&mut writer, "  }}")?;
        writeln!(&mut writer, "")?;
    }

    writeln!(&mut writer, "}}")?;

    Ok(())
}
