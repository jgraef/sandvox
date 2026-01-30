use bevy_ecs::{
    component::Component,
    reflect::ReflectComponent,
};
use bevy_reflect::{
    Reflect,
    ReflectSerialize,
};
use nalgebra::{
    Isometry3,
    Point3,
    Translation3,
    UnitQuaternion,
    UnitVector3,
    Vector3,
};
use serde::{
    Deserialize,
    Serialize,
};

#[derive(Clone, Copy, Debug, Default, Component, Reflect, Serialize, Deserialize)]
#[reflect(Component, Serialize)]
pub struct LocalTransform {
    /// Rotation followed by translation that transforms points from the
    /// object's local frame to the global frame.
    #[reflect(ignore)]
    pub isometry: Isometry3<f32>,
}

impl LocalTransform {
    #[inline]
    pub fn identity() -> Self {
        Self {
            isometry: Isometry3::identity(),
        }
    }

    #[inline]
    pub fn new(
        translation: impl Into<Translation3<f32>>,
        rotation: impl Into<UnitQuaternion<f32>>,
    ) -> Self {
        Self {
            isometry: Isometry3::from_parts(translation.into(), rotation.into()),
        }
    }

    #[inline]
    pub fn translate_local(&mut self, translation: &Translation3<f32>) {
        self.isometry.translation.vector +=
            self.isometry.rotation.transform_vector(&translation.vector);
    }

    #[inline]
    pub fn translate_global(&mut self, translation: &Translation3<f32>) {
        self.isometry.translation.vector += &translation.vector;
    }

    #[inline]
    pub fn rotate_local(&mut self, rotation: &UnitQuaternion<f32>) {
        self.isometry.rotation *= rotation;
    }

    #[inline]
    pub fn rotate_global(&mut self, rotation: &UnitQuaternion<f32>) {
        self.isometry.append_rotation_mut(rotation);
    }

    #[inline]
    pub fn rotate_around(&mut self, anchor: &Point3<f32>, rotation: &UnitQuaternion<f32>) {
        self.isometry
            .append_rotation_wrt_point_mut(rotation, anchor);
    }

    #[inline]
    pub fn look_at(eye: &Point3<f32>, target: &Point3<f32>, up: &Vector3<f32>) -> Self {
        Self {
            isometry: Isometry3::face_towards(eye, target, up),
        }
    }

    /// Pan and tilt object (e.g. a camera) with a given `up` vector.
    ///
    /// Pan is the horizontal turning. Tilt is the vertical turning.
    pub fn pan_tilt(&mut self, pan: f32, tilt: f32, up: &Vector3<f32>) {
        let local_up =
            UnitVector3::new_normalize(self.isometry.rotation.inverse_transform_vector(up));
        let local_right = Vector3::x_axis();

        let rotation = UnitQuaternion::from_axis_angle(&local_up, -pan)
            * UnitQuaternion::from_axis_angle(&local_right, tilt);

        self.isometry.rotation *= rotation;
    }

    #[inline]
    pub fn position(&self) -> Point3<f32> {
        self.isometry.translation.vector.into()
    }
}

impl From<Isometry3<f32>> for LocalTransform {
    #[inline]
    fn from(value: Isometry3<f32>) -> Self {
        Self { isometry: value }
    }
}

impl From<Translation3<f32>> for LocalTransform {
    #[inline]
    fn from(value: Translation3<f32>) -> Self {
        Self::from(Isometry3::from(value))
    }
}

impl From<Vector3<f32>> for LocalTransform {
    #[inline]
    fn from(value: Vector3<f32>) -> Self {
        Self::from(Isometry3::from(value))
    }
}

impl From<Point3<f32>> for LocalTransform {
    #[inline]
    fn from(value: Point3<f32>) -> Self {
        Self::from(value.coords)
    }
}

impl From<UnitQuaternion<f32>> for LocalTransform {
    #[inline]
    fn from(value: UnitQuaternion<f32>) -> Self {
        Self::from(Isometry3::from_parts(Default::default(), value))
    }
}
