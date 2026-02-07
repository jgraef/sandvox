use std::sync::Arc;

use bevy_ecs::{
    component::Component,
    entity::Entity,
};
use nalgebra::{
    Translation3,
    UnitQuaternion,
};

#[derive(Clone, Debug, Component)]
pub struct Animation {
    data: Arc<AnimationData>,
}

#[derive(Debug)]
struct AnimationData {
    channels: Vec<Channel>,
}

#[derive(Debug)]
pub enum Channel {
    Translation {
        target: Entity,
        key_frames: Vec<KeyFrame<Translation3<f32>>>,
    },
    Rotation {
        target: Entity,
        key_frames: Vec<KeyFrame<UnitQuaternion<f32>>>,
    },
}

#[derive(Debug)]
pub struct KeyFrame<V> {
    pub time: f32,
    pub value: V,
}

#[derive(Debug)]
struct AnimationState {
    time: f32,
    channels: Vec<ChannelState>,
}

#[derive(Debug)]
struct ChannelState {
    frame_index: usize,
}
