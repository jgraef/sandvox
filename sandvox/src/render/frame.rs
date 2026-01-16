use std::time::Instant;

use bevy_ecs::{
    change_detection::{
        DetectChanges,
        Ref,
    },
    component::Component,
    entity::Entity,
    name::NameOrEntity,
    query::{
        With,
        Without,
    },
    resource::Resource,
    system::{
        Commands,
        Local,
        Populated,
        Query,
        Res,
        ResMut,
    },
};
use bytemuck::{
    Pod,
    Zeroable,
};
use nalgebra::{
    Matrix4,
    Vector2,
};
use palette::Srgba;

use crate::{
    render::{
        staging::Staging,
        surface::{
            ClearColor,
            Surface,
        },
    },
    wgpu::{
        WgpuContext,
        buffer::WriteStaging,
    },
};

pub(super) fn create_frame_uniform_layout(wgpu: Res<WgpuContext>, mut commands: Commands) {
    let bind_group_layout =
        wgpu.device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("frame uniform"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

    commands.insert_resource(FrameUniformLayout { bind_group_layout });
}

pub(super) fn create_frames(
    wgpu: Res<WgpuContext>,
    frame_uniform_layout: Res<FrameUniformLayout>,
    surfaces: Populated<Entity, (With<Surface>, Without<Frame>)>,
    mut commands: Commands,
) {
    for entity in surfaces {
        let uniform_buffer = wgpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("frame uniform"),
            size: size_of::<FrameUniformData>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
            mapped_at_creation: false,
        });

        let uniform_bind_group = wgpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frame uniform"),
            layout: &frame_uniform_layout.bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            }],
        });

        commands.entity(entity).insert((
            Frame { inner: None },
            FrameUniform {
                bind_group: uniform_bind_group,
                buffer: uniform_buffer,
                data: Zeroable::zeroed(),
            },
        ));
    }
}

pub(super) fn begin_frames(
    wgpu: Res<WgpuContext>,
    surfaces: Populated<(&Surface, Option<&ClearColor>, &mut Frame, Ref<FrameUniform>)>,
    // todo: make it work with Res
    mut staging: ResMut<Staging>,
) {
    let start_time = Instant::now();

    for (surface, clear_color, mut frame, frame_uniform) in surfaces {
        assert!(frame.inner.is_none());

        let mut command_encoder =
            wgpu.device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("frame"),
                });

        let surface_texture = surface.surface_texture();
        let surface_texture_view =
            surface_texture
                .texture
                .create_view(&wgpu::TextureViewDescriptor {
                    label: Some("surface"),
                    ..Default::default()
                });

        let mut render_pass = command_encoder
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

        // update frame uniform buffer
        if frame_uniform.is_changed() {
            staging.write_buffer_from_slice(
                frame_uniform.buffer.slice(..),
                bytemuck::bytes_of(&frame_uniform.data),
            );
        }

        // bind frame uniform buffer
        render_pass.set_bind_group(0, Some(&frame_uniform.bind_group), &[]);

        frame.inner = Some(FrameInner {
            command_encoder,
            render_pass,
            surface_texture,
            start_time,
        });
    }
}

pub fn end_frames(
    wgpu: Res<WgpuContext>,
    frames: Query<(NameOrEntity, &mut Frame)>,
    mut command_buffers: Local<Vec<wgpu::CommandBuffer>>,
    mut present_surfaces: Local<Vec<wgpu::SurfaceTexture>>,
    mut staging: ResMut<Staging>,
) {
    assert!(command_buffers.is_empty());
    assert!(present_surfaces.is_empty());

    // flush staging. this also submits the command encoder
    command_buffers.push(staging.flush(&wgpu).finish());

    // end all render passes and get the surface textures
    for (name, mut frame) in frames {
        if let Some(FrameInner {
            command_encoder,
            render_pass,
            surface_texture,
            start_time,
        }) = frame.inner.take()
        {
            // drop the render pass explicitely since we'll submit the command encoder next
            drop(render_pass);

            // finish the frame's renderpass command encoder
            command_buffers.push(command_encoder.finish());
            // and present after we submit
            present_surfaces.push(surface_texture);

            let end_time = Instant::now();
            let time = end_time - start_time;

            tracing::trace!(surface = %name, ?time, "rendered frame");
        }
    }

    // submit all command buffers
    wgpu.queue.submit(command_buffers.drain(..));

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
    #[allow(dead_code)]
    fn inner(&self) -> &FrameInner {
        self.inner.as_ref().expect("No active frame")
    }

    fn inner_mut(&mut self) -> &mut FrameInner {
        self.inner.as_mut().expect("No active frame")
    }

    pub fn render_pass_mut(&mut self) -> &mut wgpu::RenderPass<'static> {
        &mut self.inner_mut().render_pass
    }
}

#[derive(Debug)]
struct FrameInner {
    command_encoder: wgpu::CommandEncoder,
    render_pass: wgpu::RenderPass<'static>,
    surface_texture: wgpu::SurfaceTexture,
    start_time: Instant,
}

fn srgba_to_wgpu(color: Srgba<f32>) -> wgpu::Color {
    wgpu::Color {
        r: color.red as f64,
        g: color.green as f64,
        b: color.blue as f64,
        a: color.alpha as f64,
    }
}

#[derive(Debug, Resource)]
pub struct FrameUniformLayout {
    pub bind_group_layout: wgpu::BindGroupLayout,
}

#[derive(Debug, Component)]
pub struct FrameUniform {
    bind_group: wgpu::BindGroup,
    buffer: wgpu::Buffer,
    data: FrameUniformData,
}

impl FrameUniform {
    pub fn set_viewport_size(&mut self, viewport_size: Vector2<u32>) {
        self.data.viewport_size = viewport_size;
    }

    pub fn set_camera_matrix(&mut self, camera_matrix: Matrix4<f32>) {
        self.data.camera_matrix = camera_matrix;
    }
}

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
struct FrameUniformData {
    viewport_size: Vector2<u32>,
    _padding: [u32; 2],
    camera_matrix: Matrix4<f32>,
}
