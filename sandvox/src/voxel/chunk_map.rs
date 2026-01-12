use std::collections::HashMap;

use bevy_ecs::{
    component::Component,
    entity::Entity,
    lifecycle::HookContext,
    message::{
        Message,
        MessageReader,
    },
    resource::Resource,
    system::{
        Query,
        ResMut,
    },
    world::DeferredWorld,
};
use color_eyre::eyre::Error;
use nalgebra::Point3;

use crate::ecs::{
    plugin::{
        Plugin,
        WorldBuilder,
    },
    schedule,
};

pub struct ChunkMapPlugin;

impl Plugin for ChunkMapPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder
            .add_message::<ChunkMapMessage>()
            .insert_resource(ChunkMap::default())
            .add_systems(schedule::Update, update_chunk_map);

        Ok(())
    }
}

#[derive(Debug, Default, Resource)]
pub struct ChunkMap {
    map: HashMap<Point3<i32>, Entity>,
}

impl ChunkMap {
    pub fn get(&self, position: Point3<i32>) -> Option<Entity> {
        self.map.get(&position).copied()
    }

    pub fn contains(&self, position: Point3<i32>) -> bool {
        self.map.contains_key(&position)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Component)]
#[component(on_add = chunk_added, on_remove = chunk_removed)]
pub struct ChunkPosition(pub Point3<i32>);

fn chunk_added(mut world: DeferredWorld, context: HookContext) {
    world.write_message(ChunkMapMessage::Added {
        entity: context.entity,
    });
}

fn chunk_removed(mut world: DeferredWorld, context: HookContext) {
    world.write_message(ChunkMapMessage::Removed {
        entity: context.entity,
    });
}

#[derive(Clone, Copy, Debug, Message)]
enum ChunkMapMessage {
    Added { entity: Entity },
    Removed { entity: Entity },
}

fn update_chunk_map(
    mut messages: MessageReader<ChunkMapMessage>,
    mut chunk_map: ResMut<ChunkMap>,
    chunks: Query<&mut ChunkPosition>,
) {
    for message in messages.read() {
        match message {
            ChunkMapMessage::Added { entity } => {
                let position = chunks.get(*entity).unwrap().0;
                chunk_map.map.insert(position, *entity);
            }
            ChunkMapMessage::Removed { entity } => {
                let position = chunks.get(*entity).unwrap().0;

                chunk_map.map.remove(&position);
            }
        }
    }
}
