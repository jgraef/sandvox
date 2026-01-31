use bevy_ecs::{
    component::Component,
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
    render::RenderConfig,
    wgpu::WgpuContext,
};

#[profiling::function]
pub(super) fn create_surfaces(
    wgpu: Res<WgpuContext>,
    config: Res<RenderConfig>,
    windows: Populated<(NameOrEntity, &WindowHandle, &WindowSize), Without<Surface>>,
    mut commands: Commands,
) {
    for (entity, window_handle, window_size) in windows {
        tracing::info!(%entity, "creating surface");

        let surface = Surface::new(&wgpu, &window_handle, window_size.size, &config);
        commands.entity(entity.entity).insert(surface);
    }
}

#[profiling::function]
pub(super) fn reconfigure_surfaces(
    wgpu: Res<WgpuContext>,
    windows: Populated<(&mut Surface, &WindowSize), Changed<WindowSize>>,
) {
    for (mut surface, window_size) in windows {
        surface.resize(&wgpu, window_size.size);
    }
}

#[profiling::function]
pub(super) fn set_swap_chain_texture(windows: Populated<&mut Surface>) {
    for mut surface in windows {
        surface.ensure_swap_chain_texture();
    }
}

#[profiling::function]
pub(super) fn present_surfaces(windows: Populated<&mut Surface>) {
    for mut surface in windows {
        surface.present();
    }
}

#[derive(Debug, Component)]
pub struct Surface {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    depth_texture: wgpu::TextureView,
    depth_format: wgpu::TextureFormat,
    swap_chain_texture: Option<SwapChainTexture>,
}

impl Surface {
    pub fn new(
        wgpu: &WgpuContext,
        window: &WindowHandle,
        size: Vector2<u32>,
        config: &RenderConfig,
    ) -> Self {
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

        tracing::debug!(?size, format = ?surface_texture_format, "created surface");

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
            depth_format: depth_stencil_format,
            swap_chain_texture: None,
        }
    }

    pub fn size(&self) -> Vector2<u32> {
        Vector2::new(self.config.width, self.config.height)
    }

    pub fn resize(&mut self, wgpu: &WgpuContext, size: Vector2<u32>) {
        if size != self.size() {
            tracing::debug!(?size, "resizing surface");

            self.config.width = size.x;
            self.config.height = size.y;
            self.surface.configure(&wgpu.device, &self.config);

            self.depth_texture = create_depth_texture(wgpu, size, self.depth_format);
        }
    }

    pub fn surface_texture(&self) -> &wgpu::TextureView {
        let swap_chain_texture = self.swap_chain_texture.as_ref().unwrap();
        &swap_chain_texture.texture_view
    }

    pub fn depth_texture(&self) -> &wgpu::TextureView {
        &self.depth_texture
    }

    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.config.format
    }

    pub fn depth_format(&self) -> wgpu::TextureFormat {
        self.depth_format
    }

    pub fn ensure_swap_chain_texture(&mut self) {
        if self.swap_chain_texture.is_none() {
            self.swap_chain_texture = Some(SwapChainTexture::new(&self.surface));
        }
    }

    pub fn present(&mut self) {
        if let Some(swap_chain_texture) = self.swap_chain_texture.take() {
            swap_chain_texture.surface_texture.present();
        }
    }
}

#[derive(Debug)]
struct SwapChainTexture {
    surface_texture: wgpu::SurfaceTexture,
    texture_view: wgpu::TextureView,
}

impl SwapChainTexture {
    fn new(surface: &wgpu::Surface) -> Self {
        let surface_texture = surface.get_current_texture().unwrap();
        let texture_view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor {
                label: Some("surface"),
                ..Default::default()
            });
        Self {
            surface_texture,
            texture_view,
        }
    }
}

fn create_depth_texture(
    wgpu: &WgpuContext,
    size: Vector2<u32>,
    format: wgpu::TextureFormat,
) -> wgpu::TextureView {
    let depth_texture = wgpu.device.create_texture(&wgpu::TextureDescriptor {
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
    });

    depth_texture.create_view(&wgpu::TextureViewDescriptor {
        label: Some("depth texture"),
        ..Default::default()
    })
}

#[derive(Clone, Copy, Debug, Component)]
pub struct ClearColor(pub Srgba<f32>);

impl Default for ClearColor {
    fn default() -> Self {
        Self(Srgba::new(0.0, 0.0, 0.0, 1.0))
    }
}
