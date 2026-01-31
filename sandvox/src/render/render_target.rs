use bevy_ecs::{
    component::Component,
    entity::Entity,
};

// todo: make this an enum that can be more than a window
#[derive(Clone, Copy, Debug, Component)]
#[relationship(relationship_target = RenderSources)]
pub struct RenderTarget(pub Entity);

#[derive(Clone, Debug, Component)]
#[relationship_target(relationship = RenderTarget)]
pub struct RenderSources(Vec<Entity>);
