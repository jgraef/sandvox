use bevy_ecs::{
    component::Component,
    entity::Entity,
    name::NameOrEntity,
    query::{
        Changed,
        Without,
    },
    system::{
        Commands,
        Populated,
        Res,
    },
};
use nalgebra::Vector2;
use palette::Srgba;

use crate::{
    app::{
        WindowHandle,
        WindowSize,
    },
    render::{
        RenderConfig,
        frame::FrameUniform,
    },
    wgpu::WgpuContext,
};

pub(super) fn create_surfaces(
    wgpu: Res<WgpuContext>,
    config: Res<RenderConfig>,
    windows: Populated<(NameOrEntity, &WindowHandle, &WindowSize), Without<Surface>>,
    mut commands: Commands,
) {
    for (entity, window_handle, window_size) in windows {
        tracing::info!(?entity, "creating surface");

        let surface = Surface::new(&wgpu, &window_handle, window_size.size, &config);
        commands.entity(entity.entity).insert(surface);
    }
}

// todo: this should be handled by UI I think
pub(super) fn update_viewports(
    windows: Populated<(&mut FrameUniform, &WindowSize), Changed<WindowSize>>,
) {
    for (mut frame_uniform, window_size) in windows {
        frame_uniform.set_viewport_size(window_size.size);
    }
}

pub(super) fn reconfigure_surfaces(
    wgpu: Res<WgpuContext>,
    windows: Populated<(&mut Surface, &WindowSize), Changed<WindowSize>>,
) {
    for (mut surface, window_size) in windows {
        surface.resize(&wgpu, window_size.size);
    }
}

#[derive(Debug, Component)]
pub struct Surface {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    depth_texture: wgpu::Texture,
    depth_stencil_format: wgpu::TextureFormat,
}

impl Surface {
    pub fn new(
        wgpu: &WgpuContext,
        window: &WindowHandle,
        size: Vector2<u32>,
        config: &RenderConfig,
    ) -> Self {
        tracing::debug!(?size, "creating surface");

        let surface = wgpu.instance.create_surface(window.window.clone()).unwrap();

        let capabilities = surface.get_capabilities(&wgpu.adapter);
        let surface_texture_format = *capabilities
            .formats
            .iter()
            .filter(|format| format.is_srgb())
            .next()
            .unwrap_or_else(|| {
                capabilities
                    .formats
                    .first()
                    .expect("Surface has no supported texture formats")
            });

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_texture_format,
            width: size.x,
            height: size.y,
            present_mode: if config.vsync {
                wgpu::PresentMode::AutoVsync
            }
            else {
                wgpu::PresentMode::AutoNoVsync
            },
            desired_maximum_frame_latency: 2,
            alpha_mode: wgpu::CompositeAlphaMode::Auto,
            view_formats: vec![],
        };
        surface.configure(&wgpu.device, &config);

        // do we need to pick this from a set of supported ones?
        let depth_stencil_format = wgpu::TextureFormat::Depth24Plus;
        let depth_texture = create_depth_texture(wgpu, size, depth_stencil_format);

        Self {
            surface,
            config,
            depth_texture,
            depth_stencil_format,
        }
    }

    pub fn resize(&mut self, wgpu: &WgpuContext, size: Vector2<u32>) {
        tracing::debug!(?size, "resizing surface");

        self.config.width = size.x;
        self.config.height = size.y;
        self.surface.configure(&wgpu.device, &self.config);

        self.depth_texture = create_depth_texture(wgpu, size, self.depth_stencil_format);
    }

    pub fn surface_texture(&self) -> wgpu::SurfaceTexture {
        self.surface.get_current_texture().unwrap()
    }

    pub fn depth_texture(&self) -> wgpu::TextureView {
        self.depth_texture
            .create_view(&wgpu::TextureViewDescriptor {
                label: Some("depth"),
                ..Default::default()
            })
    }

    pub fn surface_texture_format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    pub fn depth_texture_format(&self) -> wgpu::TextureFormat {
        self.depth_stencil_format
    }
}

fn create_depth_texture(
    wgpu: &WgpuContext,
    size: Vector2<u32>,
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
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
        format,
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
