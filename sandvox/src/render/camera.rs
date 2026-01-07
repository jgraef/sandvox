use std::f32::consts::FRAC_PI_4;

use bevy_ecs::component::Component;
use nalgebra::{
    Perspective3,
    Point2,
    Point3,
    Vector2,
};

#[derive(Clone, Copy, Debug, Component)]
pub struct CameraProjection {
    // note: not public because nalgebra seems to have the z-axis inverted relative to our
    // coordinate systems
    projection: Perspective3<f32>,
}

impl CameraProjection {
    /// # Arguments
    ///
    /// - `fovy`: Field of view along (camera-local) Y-axis (vertical angle).
    pub fn new(fovy: f32) -> Self {
        let projection = Perspective3::new(1.0, fovy, 0.1, 100.0);
        Self { projection }
    }

    pub(super) fn set_viewport(&mut self, viewport: Vector2<u32>) {
        if viewport.y != 0 {
            let viewport = viewport.cast::<f32>();
            self.set_aspect_ratio(viewport.x / viewport.y);
        }
    }

    /// Set aspect ratio (width / height)
    pub fn set_aspect_ratio(&mut self, aspect_ratio: f32) {
        self.projection.set_aspect(aspect_ratio);
    }

    pub fn unproject(&self, point: &Point3<f32>) -> Point3<f32> {
        let mut point = self.projection.unproject_point(point);
        // nalgebra's projection uses a reversed z-axis
        point.z *= -1.0;
        point
    }

    /// Returns angles (horizontal, vertical) that a point makes with the focal
    /// point of the camera.
    pub fn unproject_screen(&self, point: &Point2<f32>) -> Vector2<f32> {
        let fovy = self.projection.fovy();
        let aspect_ratio = self.projection.aspect();
        Vector2::new(point.x * fovy / aspect_ratio, point.y * fovy)
    }

    pub fn fovy(&self) -> f32 {
        self.projection.fovy()
    }

    /// Aspect ration (width / height)
    pub fn aspect_ratio(&self) -> f32 {
        self.projection.aspect()
    }
}

impl Default for CameraProjection {
    fn default() -> Self {
        // 45 degrees
        let fovy = FRAC_PI_4;

        Self::new(fovy)
    }
}
