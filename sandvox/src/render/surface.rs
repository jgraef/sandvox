use bevy_ecs::{
    component::Component,
    entity::Entity,
    message::MessageReader,
    system::{
        Commands,
        ParamSet,
        Query,
        Res,
    },
};
use nalgebra::Vector2;
use palette::Srgba;

use crate::{
    app::{
        WindowEvent,
        WindowHandle,
        WindowSize,
    },
    render::camera::CameraProjection,
    wgpu::WgpuContext,
};

pub fn handle_window_events(
    wgpu: Res<WgpuContext>,
    mut messages: MessageReader<WindowEvent>,
    mut params: ParamSet<(
        (
            Query<(&WindowHandle, &WindowSize, Option<&AttachedCamera>)>,
            Query<&mut CameraProjection>,
        ),
        (
            Query<(&mut Surface, &WindowSize, Option<&AttachedCamera>)>,
            Query<&mut CameraProjection>,
        ),
    )>,
    mut commands: Commands,
) {
    for message in messages.read() {
        match message {
            WindowEvent::Created { window } => {
                let entity = *window;
                let (windows, mut cameras) = params.p0();

                let (window, window_size, camera) = windows.get(entity).unwrap();

                let surface = Surface::new(&wgpu, window, window_size.size);
                commands.entity(entity).insert(surface);

                if let Some(camera) = camera
                    && let Ok(mut camera) = cameras.get_mut(camera.0)
                {
                    camera.set_viewport(window_size.size);
                }
            }
            WindowEvent::Resized { window, size } => {
                let (mut surfaces, mut cameras) = params.p1();
                let (mut surface, window_size, camera) = surfaces.get_mut(*window).unwrap();

                surface.resize(&wgpu, *size);

                if let Some(camera) = camera
                    && let Ok(mut camera) = cameras.get_mut(camera.0)
                {
                    camera.set_viewport(window_size.size);
                }
            }
        }
    }
}

#[derive(Debug, Component)]
pub struct Surface {
    surface: wgpu::Surface<'static>,
    depth_texture: wgpu::Texture,
    config: wgpu::SurfaceConfiguration,
}

impl Surface {
    pub fn new(wgpu: &WgpuContext, window: &WindowHandle, size: Vector2<u32>) -> Self {
        tracing::debug!(?size, "creating surface");

        let surface = wgpu.instance.create_surface(window.window.clone()).unwrap();

        let config = surface
            .get_default_config(&wgpu.adapter, size.x, size.y)
            .expect("surface not supported by adpater");
        surface.configure(&wgpu.device, &config);

        let depth_texture = create_depth_texture(wgpu, size);

        Self {
            surface,
            depth_texture,
            config,
        }
    }

    pub fn resize(&mut self, wgpu: &WgpuContext, size: Vector2<u32>) {
        tracing::debug!(?size, "resizing surface");

        self.config.width = size.x;
        self.config.height = size.y;
        self.surface.configure(&wgpu.device, &self.config);

        self.depth_texture = create_depth_texture(wgpu, size)
    }

    pub fn get_surface_texture(&self) -> wgpu::SurfaceTexture {
        self.surface.get_current_texture().unwrap()
    }

    pub fn get_depth_texture(&self) -> wgpu::TextureView {
        self.depth_texture
            .create_view(&wgpu::TextureViewDescriptor {
                label: Some("depth"),
                ..Default::default()
            })
    }
}

fn create_depth_texture(wgpu: &WgpuContext, size: Vector2<u32>) -> wgpu::Texture {
    wgpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("depth texture"),
        size: wgpu::Extent3d {
            width: size.x,
            height: size.y,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TRANSIENT,
        view_formats: &[],
    })
}

#[derive(Clone, Copy, Debug, Component)]
pub struct ClearColor(pub Srgba<f32>);

impl Default for ClearColor {
    fn default() -> Self {
        Self(Srgba::new(0.0, 0.0, 0.0, 1.0))
    }
}

#[derive(Clone, Copy, Debug, Component)]
pub struct AttachedCamera(pub Entity);
