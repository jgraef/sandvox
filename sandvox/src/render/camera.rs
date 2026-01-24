use bevy_ecs::{
    component::Component,
    entity::Entity,
    query::{
        Changed,
        Or,
        Without,
    },
    relationship::RelationshipTarget,
    schedule::IntoScheduleConfigs,
    system::{
        Commands,
        Populated,
    },
};
use color_eyre::eyre::Error;
use nalgebra::{
    Isometry3,
    Matrix4,
    Point2,
    Vector2,
    Vector4,
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
        frame::{
            CameraData,
            FrameUniform,
        },
        surface::{
            RenderSources,
            RenderTarget,
        },
    },
};

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder.add_systems(
            schedule::Render,
            (
                update_cameras,
                (update_camera_projections, update_camera_frustrums).after(update_cameras),
                update_camera_matrices
                    .before(RenderSystems::EndFrame)
                    .after(update_camera_projections),
            ),
        );

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Component)]
pub struct Camera {
    pub aspect_ratio: f32,
    pub fovy: f32,
    pub z_near: f32,
    pub z_far: f32,
}

impl Camera {
    pub fn set_viewport(&mut self, viewport: Vector2<u32>) {
        if viewport.y != 0 {
            let viewport = viewport.cast::<f32>();
            self.aspect_ratio = viewport.x / viewport.y;
        }
    }

    pub fn projection(&self) -> CameraProjection {
        CameraProjection::new(self.aspect_ratio, self.fovy, self.z_near, self.z_far)
    }

    pub fn frustrum(&self) -> CameraFrustrum {
        let half_fov = 0.5 * self.fovy;

        CameraFrustrum {
            frustrum: Frustrum::new(
                half_fov * self.aspect_ratio,
                half_fov,
                self.z_near,
                self.z_far,
            ),
        }
    }

    /// Returns angles (horizontal, vertical) that a point makes with the focal
    /// point of the camera.
    pub fn unproject_screen(&self, point: &Point2<f32>) -> Vector2<f32> {
        Vector2::new(point.x * self.fovy / self.aspect_ratio, point.y * self.fovy)
    }
}

/// Camera projection matrix
///
/// Suitable for wgpu, so that [z_near, z_far] maps to [0, 1] with a non-linear
/// mapping.
///
/// Derived with help of [this website][1]. Basically you set `z' = -c1/z + c2`
/// such that `z_near -> 0` and `z_far -> 1` and solve for `c1` and `c2`:
///
/// `c2 = z_var / (z_near - z_far)`
///
/// `c1 = z_near * c2`
///
/// The projection matrix then is:
///
/// ```plain
///  s/a 0   0  0
///    0 s   0  0
///    0 0 -c2 c1
///    0 0   1  0
/// ```
///
/// and its inverse is:
///
/// ```plain
/// a/s   0    0     0
///   0 1/s    0     0
///   0   0    0     1
///   0   0 1/c1 -1/c2
/// ```
///
/// where `s = 1 / tan(fovy /2)` and a is the aspect ratio.
///
/// [1]: https://learnwebgl.brown37.net/08_projections/projections_perspective.html
#[derive(Clone, Copy, Debug, Component)]
pub struct CameraProjection {
    a: f32,
    s: f32,
    c1: f32,
    c2: f32,
}

impl CameraProjection {
    pub fn new(aspect_ratio: f32, fovy: f32, z_near: f32, z_far: f32) -> Self {
        let s = 1.0 / (0.5 * fovy).tan();
        let c2 = z_far / (z_near - z_far);
        let c1 = z_near * c2;

        Self {
            a: aspect_ratio,
            s,
            c1,
            c2,
        }
    }

    pub fn project(&self, vector: Vector4<f32>) -> Vector4<f32> {
        // todo: test
        Vector4::new(
            vector.x * self.s / self.a,
            vector.y * self.s,
            vector.z * -self.c2 + vector.w * self.c1,
            vector.z,
        )
    }

    pub fn unproject(&self, vector: Vector4<f32>) -> Vector4<f32> {
        // todo: test
        let s_inv = 1.0 / self.s;
        Vector4::new(
            vector.x * self.a * s_inv,
            vector.y * s_inv,
            vector.w,
            vector.z / self.c1 - vector.w / self.c2,
        )
    }

    pub fn to_matrix(&self) -> Matrix4<f32> {
        let mut matrix = Matrix4::zeros();
        matrix.m11 = self.s / self.a;
        matrix.m22 = self.s;
        matrix.m33 = -self.c2;
        matrix.m34 = self.c1;
        matrix.m43 = 1.0;
        matrix
    }

    pub fn to_inverse(&self) -> Matrix4<f32> {
        let mut matrix_inv = Matrix4::zeros();
        matrix_inv.m11 = self.a / self.s;
        matrix_inv.m22 = 1.0 / self.s;
        matrix_inv.m34 = 1.0;
        matrix_inv.m43 = 1.0 / self.c1;
        matrix_inv.m44 = -1.0 / self.c2;
        matrix_inv
    }
}

fn update_cameras(
    windows: Populated<(&WindowSize, &RenderSources), Changed<WindowSize>>,
    mut cameras: Populated<&mut Camera>,
) {
    for (window_size, render_sources) in windows {
        for entity in render_sources.iter() {
            if let Ok(mut projection) = cameras.get_mut(entity) {
                projection.set_viewport(window_size.size);
            }
        }
    }
}

fn update_camera_projections(
    cameras: Populated<
        (Entity, &Camera, Option<&mut CameraProjection>),
        Or<(Changed<CameraProjection>, Without<CameraProjection>)>,
    >,
    mut commands: Commands,
) {
    for (entity, camera, projection) in cameras {
        if let Some(mut projection) = projection {
            *projection = camera.projection();
        }
        else {
            commands.entity(entity).insert(camera.projection());
        }
    }
}

fn update_camera_matrices(
    cameras: Populated<
        (&CameraProjection, &GlobalTransform, &RenderTarget),
        Or<(
            Changed<CameraProjection>,
            Changed<GlobalTransform>,
            Changed<RenderTarget>,
        )>,
    >,
    mut frame_uniforms: Populated<&mut FrameUniform>,
) {
    for (projection, transform, render_target) in cameras {
        if let Ok(mut frame_uniform) = frame_uniforms.get_mut(render_target.0) {
            frame_uniform.data.camera = CameraData {
                projection: projection.to_matrix(),
                projection_inverse: projection.to_inverse(),
                view: transform.isometry().inverse().to_homogeneous(),
                view_inverse: transform.isometry().to_homogeneous(),
                position: transform.position().to_homogeneous(),
            };
        }
    }
}

fn update_camera_frustrums(
    changed: Populated<
        (Entity, &Camera, Option<&mut CameraFrustrum>),
        Or<(
            Changed<Camera>,
            Changed<GlobalTransform>,
            Without<CameraFrustrum>,
        )>,
    >,
    mut commands: Commands,
) {
    for (entity, projection, frustrum) in changed {
        // update frustrum
        if let Some(mut frustrum) = frustrum {
            *frustrum = projection.frustrum();
        }
        else {
            commands.entity(entity).insert(projection.frustrum());
        }
    }
}

#[derive(Clone, Copy, Debug, Component)]
pub struct CameraFrustrum {
    pub frustrum: Frustrum,
}

impl CameraFrustrum {
    pub fn cull(&self, isometry: &Isometry3<f32>, aabb: &Aabb) -> bool {
        !self.frustrum.intersect_aabb(isometry, aabb)
    }
}

#[derive(Clone, Copy, Debug, Component)]
pub struct FrustrumCulled {
    pub aabb: Aabb,
}

#[cfg(test)]
mod tests {
    use nalgebra::{
        Perspective3,
        Point3,
    };

    #[test]
    fn how_tf_does_nalgebras_perspective_map() {
        let perspective = Perspective3::new(1.0, 60.0f32.to_radians(), 0.1, 100.0).to_homogeneous();
        println!("{perspective:#?}");

        let point = Point3::new(0.0, 0.0, 100.0).to_homogeneous();
        let projected_point = Point3::from_homogeneous(perspective * point).unwrap();

        assert_eq!(projected_point, Point3::new(0.0, 0.0, 1.0));
    }
}
