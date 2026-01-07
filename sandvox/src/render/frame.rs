use bevy_ecs::{
    component::Component,
    entity::Entity,
    system::{
        Commands,
        Query,
        Res,
    },
};
use palette::Srgba;

use crate::{
    render::surface::{
        AttachedCamera,
        ClearColor,
        Surface,
    },
    wgpu::WgpuContext,
};

pub fn begin_frame(
    wgpu: Res<WgpuContext>,
    surfaces: Query<(
        Entity,
        &Surface,
        Option<&ClearColor>,
        Option<&AttachedCamera>,
    )>,
    mut commands: Commands,
) {
    tracing::debug!("begin frame");

    for (surface_entity, surface, clear_color, camera) in surfaces {
        tracing::debug!(?surface_entity, "rendering");

        let mut command_encoder =
            wgpu.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("frame"),
                });

        let surface_texture = surface.get_surface_texture();
        let surface_texture_view =
            surface_texture
                .texture
                .create_view(&wgpu::TextureViewDescriptor {
                    label: Some("surface"),
                    ..Default::default()
                });

        let render_pass = command_encoder
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
                    view: &surface.get_depth_texture(),
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
        let _ = camera;

        commands.entity(surface_entity).insert(Frame {
            inner: Some(FrameInner {
                command_encoder,
                render_pass,
                surface_texture,
            }),
        });
    }
}

pub fn end_frame(wgpu: Res<WgpuContext>, frames: Query<&mut Frame>) {
    for mut frame in frames {
        if let Some(frame) = frame.inner.take() {
            tracing::debug!("end_frame");
            // first drop the render pass such that it doesn't "block" the command encoder
            // anymore
            drop(frame.render_pass);

            // submit command buffer
            let command_buffer = frame.command_encoder.finish();
            wgpu.queue.submit([command_buffer]);

            frame.surface_texture.present();
        }
    }
}

#[derive(Debug, Component)]
pub struct Frame {
    inner: Option<FrameInner>,
}

#[derive(Debug)]
struct FrameInner {
    command_encoder: wgpu::CommandEncoder,
    render_pass: wgpu::RenderPass<'static>,
    surface_texture: wgpu::SurfaceTexture,
}

fn srgba_to_wgpu(color: Srgba<f32>) -> wgpu::Color {
    wgpu::Color {
        r: color.red as f64,
        g: color.green as f64,
        b: color.blue as f64,
        a: color.alpha as f64,
    }
}
