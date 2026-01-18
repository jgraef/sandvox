use std::f32::consts::FRAC_PI_4;

use bevy_ecs::{
    change_detection::{
        DetectChanges,
        Ref,
    },
    component::Component,
    entity::Entity,
    query::{
        Changed,
        Or,
    },
    schedule::IntoScheduleConfigs,
    system::{
        Commands,
        Populated,
    },
};
use color_eyre::eyre::Error;
use nalgebra::{
    Isometry3,
    Perspective3,
    Point2,
    Point3,
    Vector2,
};

use crate::{
    app::WindowSize,
    collide::{
        Aabb,
        frustrum::Frustrum,
    },
    ecs::{
        plugin::{
            Plugin,
            WorldBuilder,
        },
        schedule,
        transform::GlobalTransform,
    },
    render::{
        RenderSystems,
        frame::FrameUniform,
        surface::AttachedCamera,
    },
};

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder.add_systems(
            schedule::Render,
            (
                update_camera_projection,
                (update_camera_matrices, update_camera_frustrums)
                    .before(RenderSystems::BeginFrame)
                    .after(update_camera_projection),
            ),
        );

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Component)]
pub struct CameraProjection {
    // note: not public because nalgebra seems to have the z-axis inverted relative to our
    // coordinate systems
    projection: Perspective3<f32>,
    fovy: f32,
    aspect_ratio: f32,
    z_bounds: [f32; 2],
}

impl CameraProjection {
    // 45 degrees
    pub const DEFAULT_FOVY: f32 = FRAC_PI_4;

    /// # Arguments
    ///
    /// - `fovy`: Field of view along (camera-local) Y-axis (vertical angle).
    pub fn new(fovy: f32, z_far: f32) -> Self {
        let z_bounds = [0.1, z_far];
        let projection = Perspective3::new(1.0, fovy, z_bounds[0], z_bounds[1]);
        Self {
            projection,
            fovy,
            aspect_ratio: 1.0,
            z_bounds,
        }
    }

    pub fn set_viewport(&mut self, viewport: Vector2<u32>) {
        if viewport.y != 0 {
            let viewport = viewport.cast::<f32>();
            self.set_aspect_ratio(viewport.x / viewport.y);
        }
    }

    /// Set aspect ratio (width / height)
    pub fn set_aspect_ratio(&mut self, aspect_ratio: f32) {
        self.aspect_ratio = aspect_ratio;
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

    pub fn fov(&self) -> Vector2<f32> {
        Vector2::new(self.aspect_ratio * self.fovy, self.fovy)
    }

    /// Aspect ration (width / height)
    pub fn aspect_ratio(&self) -> f32 {
        self.aspect_ratio
    }

    pub fn z_bounds(&self) -> [f32; 2] {
        self.z_bounds
    }
}

impl Default for CameraProjection {
    fn default() -> Self {
        Self::new(Self::DEFAULT_FOVY, 1000.0)
    }
}

fn update_camera_projection(
    windows: Populated<(&WindowSize, &AttachedCamera), Changed<WindowSize>>,
    mut cameras: Populated<&mut CameraProjection>,
) {
    for (window_size, camera) in windows {
        if let Ok(mut projection) = cameras.get_mut(camera.0) {
            projection.set_viewport(window_size.size);
        }
    }
}

fn update_camera_matrices(
    cameras: Populated<(Ref<CameraProjection>, Ref<GlobalTransform>)>,
    frame_uniforms: Populated<(&mut FrameUniform, Ref<AttachedCamera>)>,
) {
    for (mut frame_uniform, attached_camera) in frame_uniforms {
        if let Ok((projection, transform)) = cameras.get(attached_camera.0) {
            if attached_camera.is_changed() || projection.is_changed() || transform.is_changed() {
                let camera_matrix = {
                    let transform = transform.isometry().inverse().to_homogeneous();

                    let projection = {
                        let mut projection = projection.projection.to_homogeneous();
                        // nalgebra assumes we're using a right-handed world coordinate system and a
                        // left-handed NDC and thus flips the z-axis. Undo this here.
                        projection[(2, 2)] *= -1.0;
                        projection[(3, 2)] = 1.0;
                        projection
                    };

                    projection * transform
                };

                frame_uniform.set_camera_matrix(camera_matrix);
            }
        }
    }
}

fn update_camera_frustrums(
    changed: Populated<
        (Entity, &CameraProjection, Option<&mut CameraFrustrum>),
        Or<(Changed<CameraProjection>, Changed<GlobalTransform>)>,
    >,
    mut commands: Commands,
) {
    for (entity, projection, frustrum) in changed {
        // update frustrum
        if let Some(mut frustrum) = frustrum {
            *frustrum = CameraFrustrum::new(projection);
        }
        else {
            let frustrum = CameraFrustrum::new(projection);
            commands.entity(entity).insert(frustrum);
        }
    }
}

#[derive(Clone, Copy, Debug, Component)]
pub struct CameraFrustrum {
    frustrum: Frustrum,
}

impl CameraFrustrum {
    pub fn new(projection: &CameraProjection) -> Self {
        let half_fov = 0.5 * projection.fov();
        let [z_near, z_far] = projection.z_bounds();

        Self {
            frustrum: Frustrum::new(half_fov.x, half_fov.y, z_near, z_far),
        }
    }

    pub fn cull(&self, isometry: &Isometry3<f32>, aabb: &Aabb) -> bool {
        !self.frustrum.intersect_aabb(isometry, aabb)
    }
}

#[derive(Clone, Copy, Debug, Component)]
pub struct FrustrumCulled {
    pub aabb: Aabb,
}
