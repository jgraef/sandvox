use std::time::Instant;

use bevy_ecs::{
    component::Component,
    entity::Entity,
    name::NameOrEntity,
    system::{
        Commands,
        Local,
        Query,
        Res,
        ResMut,
    },
};
use palette::Srgba;

use crate::{
    render::{
        camera::CameraBindGroup,
        flush_staging,
        staging::Staging,
        surface::{
            AttachedCamera,
            ClearColor,
            Surface,
        },
    },
    wgpu::WgpuContext,
};

pub fn begin_frame(
    surfaces: Query<(
        Entity,
        &Surface,
        Option<&ClearColor>,
        Option<&AttachedCamera>,
    )>,
    cameras: Query<&CameraBindGroup>,
    // todo: make it work with Res
    mut staging: ResMut<Staging>,
    mut commands: Commands,
) {
    let start_time = Instant::now();

    for (surface_entity, surface, clear_color, camera) in surfaces {
        /*let mut command_encoder =
        wgpu.device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("frame"),
            });*/

        let surface_texture = surface.surface_texture();
        let surface_texture_view =
            surface_texture
                .texture
                .create_view(&wgpu::TextureViewDescriptor {
                    label: Some("surface"),
                    ..Default::default()
                });

        let mut render_pass = staging
            .command_encoder_mut()
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("frame"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &surface_texture_view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: clear_color.map_or(wgpu::LoadOp::Load, |color| {
                            wgpu::LoadOp::Clear(srgba_to_wgpu(color.0))
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &surface.depth_texture(),
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            })
            .forget_lifetime();

        // todo: update camera uniform buffer and bind it
        let mut has_camera = false;
        if let Some(camera) = camera
            && let Ok(camera_bind_group) = cameras.get(camera.0)
        {
            render_pass.set_bind_group(0, Some(&camera_bind_group.bind_group), &[]);
            has_camera = true;
        }

        // debug
        assert!(has_camera, "frame without camera");

        commands.entity(surface_entity).insert(Frame {
            inner: Some(FrameInner {
                //command_encoder,
                render_pass,
                surface_texture,
                start_time,
                has_camera,
            }),
        });
    }
}

pub fn end_frame(
    wgpu: Res<WgpuContext>,
    frames: Query<(NameOrEntity, &mut Frame)>,
    mut present_surfaces: Local<Vec<wgpu::SurfaceTexture>>,
    staging: ResMut<Staging>,
) {
    assert!(present_surfaces.is_empty());

    // end all render passes and get the surface textures
    present_surfaces.extend(frames.into_iter().filter_map(|(name, mut frame)| {
        frame.inner.take().map(
            |FrameInner {
                 render_pass,
                 surface_texture,
                 start_time,
                 has_camera: _,
             }| {
                // drop the render pass explicitely since we'll submit the command encoder next
                drop(render_pass);

                let end_time = Instant::now();
                let time = end_time - start_time;
                tracing::trace!(surface = %name, ?time, "rendered frame");

                surface_texture
            },
        )
    }));

    // flush staging. this also submits the command encoder
    flush_staging(wgpu, staging);

    // present surfaces
    for surface_texture in present_surfaces.drain(..) {
        surface_texture.present();
    }
}

#[derive(Debug, Component)]
pub struct Frame {
    inner: Option<FrameInner>,
}

impl Frame {
    fn inner(&self) -> &FrameInner {
        self.inner.as_ref().expect("No active frame")
    }

    fn inner_mut(&mut self) -> &mut FrameInner {
        self.inner.as_mut().expect("No active frame")
    }

    pub fn render_pass_mut(&mut self) -> &mut wgpu::RenderPass<'static> {
        &mut self.inner_mut().render_pass
    }

    pub fn has_camera(&self) -> bool {
        self.inner().has_camera
    }
}

#[derive(Debug)]
struct FrameInner {
    //command_encoder: wgpu::CommandEncoder,
    render_pass: wgpu::RenderPass<'static>,
    surface_texture: wgpu::SurfaceTexture,
    start_time: Instant,
    has_camera: bool,
}

fn srgba_to_wgpu(color: Srgba<f32>) -> wgpu::Color {
    wgpu::Color {
        r: color.red as f64,
        g: color.green as f64,
        b: color.blue as f64,
        a: color.alpha as f64,
    }
}
