use std::f32::consts::FRAC_PI_4;

use bevy_ecs::{
    component::Component,
    entity::Entity,
    query::{
        Changed,
        Or,
    },
    resource::Resource,
    schedule::IntoScheduleConfigs,
    system::{
        Commands,
        Query,
        Res,
    },
};
use bytemuck::{
    Pod,
    Zeroable,
    bytes_of,
};
use color_eyre::eyre::Error;
use nalgebra::{
    Isometry3,
    Matrix4,
    Perspective3,
    Point2,
    Point3,
    Vector2,
};
use wgpu::util::DeviceExt;

use crate::{
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
    render::RenderSystems,
    wgpu::WgpuContext,
};

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder
            .add_systems(
                schedule::Startup,
                create_camera_bind_group_layout.in_set(RenderSystems::Setup),
            )
            .add_systems(
                schedule::Render,
                (update_camera_bind_groups, update_camera_frustrums)
                    .before(RenderSystems::BeginFrame),
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

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
struct CameraData {
    matrix: Matrix4<f32>,
}

impl CameraData {
    fn new(projection: &CameraProjection, transform: &Isometry3<f32>) -> Self {
        let transform = transform.inverse().to_homogeneous();

        let projection = {
            let mut projection = projection.projection.to_homogeneous();
            // nalgebra assumes we're using a right-handed world coordinate system and a
            // left-handed NDC and thus flips the z-axis. Undo this here.
            projection[(2, 2)] *= -1.0;
            projection[(3, 2)] = 1.0;
            projection
        };

        let matrix = projection * transform;

        Self { matrix }
    }
}

#[derive(Clone, Debug, Component)]
pub struct CameraBindGroup {
    pub buffer: wgpu::Buffer,
    pub bind_group: wgpu::BindGroup,
}

impl CameraBindGroup {
    fn new(
        wgpu: &WgpuContext,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        data: &CameraData,
    ) -> Self {
        let buffer = wgpu
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("camera"),
                contents: bytemuck::bytes_of(data),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            });

        let bind_group = wgpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("camera"),
            layout: camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        });

        Self { buffer, bind_group }
    }

    fn update(&self, wgpu: &WgpuContext, data: &CameraData) {
        wgpu.queue.write_buffer(&self.buffer, 0, bytes_of(data));
    }
}

#[derive(Clone, Debug, Resource)]
pub struct CameraBindGroupLayout {
    pub bind_group_layout: wgpu::BindGroupLayout,
}

fn create_camera_bind_group_layout(wgpu: Res<WgpuContext>, mut commands: Commands) {
    let bind_group_layout =
        wgpu.device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("camera"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

    commands.insert_resource(CameraBindGroupLayout { bind_group_layout });
}

fn update_camera_bind_groups(
    wgpu: Res<WgpuContext>,
    camera_bind_group_layout: Res<CameraBindGroupLayout>,
    changed: Query<
        (
            Entity,
            &CameraProjection,
            &GlobalTransform,
            Option<&CameraBindGroup>,
        ),
        Or<(Changed<CameraProjection>, Changed<GlobalTransform>)>,
    >,
    mut commands: Commands,
) {
    for (entity, projection, transform, bind_group) in changed {
        // update bind group
        let data = CameraData::new(&projection, transform.isometry());
        if let Some(bind_group) = bind_group {
            bind_group.update(&wgpu, &data);
        }
        else {
            let bind_group =
                CameraBindGroup::new(&wgpu, &camera_bind_group_layout.bind_group_layout, &data);
            commands.entity(entity).insert(bind_group);
        }
    }
}

fn update_camera_frustrums(
    changed: Query<
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
