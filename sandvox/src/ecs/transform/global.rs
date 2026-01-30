use bevy_ecs::{
    component::Component,
    reflect::ReflectComponent,
};
use bevy_reflect::Reflect;
use nalgebra::{
    Isometry3,
    Point3,
};
use serde::{
    Deserialize,
    Serialize,
};

use crate::ecs::transform::LocalTransform;

#[derive(Clone, Copy, Debug, PartialEq, Component, Reflect, Serialize, Deserialize)]
#[reflect(Component)]
pub struct GlobalTransform {
    #[reflect(ignore)]
    pub isometry: Isometry3<f32>,
}

impl GlobalTransform {
    #[inline]
    pub fn identity() -> Self {
        Self {
            isometry: Isometry3::identity(),
        }
    }

    #[inline]
    pub fn with_local(self, local: &LocalTransform) -> Self {
        Self {
            isometry: self.isometry * local.isometry,
        }
    }

    #[inline]
    pub fn position(&self) -> Point3<f32> {
        self.isometry.translation.vector.into()
    }
}

impl From<LocalTransform> for GlobalTransform {
    #[inline]
    fn from(value: LocalTransform) -> Self {
        Self {
            isometry: value.isometry,
        }
    }
}
