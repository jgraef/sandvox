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
        Changed,
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
        RenderConfig,
        atlas::{
            Atlas,
            AtlasResources,
        },
        staging::Staging,
        surface::{
            ClearColor,
            Surface,
        },
        text::{
            Font,
            FontResources,
        },
    },
    wgpu::{
        WgpuContext,
        buffer::WriteStaging,
    },
};

pub(super) fn create_frame_bind_group_layout(wgpu: Res<WgpuContext>, mut commands: Commands) {
    let bind_group_layout =
        wgpu.device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("frame uniform"),
                entries: &[
                    // frame uniform. contains viewport size, camera matrix, etc.
                    wgpu::BindGroupLayoutEntry {
                        binding: 0,
                        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Uniform,
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // default sampler
                    wgpu::BindGroupLayoutEntry {
                        binding: 1,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                        count: None,
                    },
                    // atlas texture
                    wgpu::BindGroupLayoutEntry {
                        binding: 2,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    // atlas data
                    wgpu::BindGroupLayoutEntry {
                        binding: 3,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                    // font texture
                    wgpu::BindGroupLayoutEntry {
                        binding: 4,
                        visibility: wgpu::ShaderStages::FRAGMENT,
                        ty: wgpu::BindingType::Texture {
                            sample_type: wgpu::TextureSampleType::Float { filterable: true },
                            view_dimension: wgpu::TextureViewDimension::D2,
                            multisampled: false,
                        },
                        count: None,
                    },
                    // font glyph data
                    wgpu::BindGroupLayoutEntry {
                        binding: 5,
                        visibility: wgpu::ShaderStages::VERTEX,
                        ty: wgpu::BindingType::Buffer {
                            ty: wgpu::BufferBindingType::Storage { read_only: true },
                            has_dynamic_offset: false,
                            min_binding_size: None,
                        },
                        count: None,
                    },
                ],
            });

    commands.insert_resource(FrameBindGroupLayout { bind_group_layout });
}

pub(super) fn create_frames(
    wgpu: Res<WgpuContext>,
    frame_bind_group_layout: Res<FrameBindGroupLayout>,
    surfaces: Populated<Entity, (With<Surface>, Without<Frame>)>,
    default_sampler: Res<DefaultSampler>,
    default_atlas: Res<DefaultAtlas>,
    default_font: Res<DefaultFont>,
    mut commands: Commands,
) {
    for entity in surfaces {
        let frame_uniform = {
            let buffer = wgpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("frame uniform"),
                size: size_of::<FrameUniformData>() as wgpu::BufferAddress,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
                mapped_at_creation: false,
            });

            FrameUniform {
                buffer,
                data: Zeroable::zeroed(),
            }
        };

        let frame_bind_group = FrameBindGroup::new(
            &wgpu.device,
            &frame_bind_group_layout,
            &frame_uniform,
            &default_sampler,
            default_atlas.0.resources(),
            default_font.0.resources(),
        );

        commands
            .entity(entity)
            .insert((Frame { inner: None }, frame_uniform, frame_bind_group));
    }
}

pub(super) fn begin_frames(
    wgpu: Res<WgpuContext>,
    surfaces: Populated<(
        &Surface,
        Option<&ClearColor>,
        &mut Frame,
        Ref<FrameBindGroup>,
    )>,
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

    // todo: put this in its own systems.
    // we can just collect command buffers in a resource (i.e. this one and the ones
    // from the frames) and submit them to the queue in another system that runs
    // last. or we could submit stuff immediately i guess.
    if staging.is_changed() {
        // flush staging. this also submits the command encoder
        command_buffers.push(staging.flush(&wgpu).finish());
    }

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
pub struct FrameBindGroupLayout {
    pub bind_group_layout: wgpu::BindGroupLayout,
}

#[derive(Debug, Component)]
pub struct FrameBindGroup {
    bind_group: wgpu::BindGroup,
}

impl FrameBindGroup {
    fn new(
        device: &wgpu::Device,
        frame_bind_group_layout: &FrameBindGroupLayout,
        frame_uniform: &FrameUniform,
        default_sampler: &DefaultSampler,
        atlas_resources: AtlasResources,
        font_resources: FontResources,
    ) -> Self {
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("frame bind group"),
            layout: &frame_bind_group_layout.bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: frame_uniform.buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&default_sampler.0),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(atlas_resources.texture),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Buffer(
                        atlas_resources.data_buffer.as_entire_buffer_binding(),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(font_resources.texture),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Buffer(
                        font_resources.data_buffer.as_entire_buffer_binding(),
                    ),
                },
            ],
        });

        Self { bind_group }
    }
}

#[derive(Debug, Component)]
pub struct FrameUniform {
    buffer: wgpu::Buffer,
    pub data: FrameUniformData,
}

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
pub struct FrameUniformData {
    pub viewport_size: Vector2<u32>,
    _padding: [u32; 2],
    pub camera_matrix: Matrix4<f32>,
}

pub(super) fn update_frame_uniform(
    changed_frame_uniforms: Populated<&FrameUniform, Changed<FrameUniform>>,
    mut staging: ResMut<Staging>,
) {
    for frame_uniform in changed_frame_uniforms {
        // update frame uniform buffer
        staging.write_buffer_from_slice(
            frame_uniform.buffer.slice(..),
            bytemuck::bytes_of(&frame_uniform.data),
        );
    }
}

pub(super) fn update_frame_bind_groups(
    wgpu: Res<WgpuContext>,
    frame_bind_groups: Query<(&mut FrameBindGroup, &FrameUniform)>,
    mut atlas: ResMut<DefaultAtlas>,
    font: Res<DefaultFont>,
    default_sampler: Res<DefaultSampler>,
    mut staging: ResMut<Staging>,
    frame_bind_group_layout: Res<FrameBindGroupLayout>,
) {
    if atlas.0.flush(&wgpu.device, &mut *staging) {
        let atlas_resources = atlas.0.resources();
        let font_resources = font.0.resources();

        for (mut frame_bind_group, frame_uniform) in frame_bind_groups {
            // recreate the bind group
            *frame_bind_group = FrameBindGroup::new(
                &wgpu.device,
                &frame_bind_group_layout,
                frame_uniform,
                &default_sampler,
                atlas_resources,
                font_resources,
            )
        }
    }
}

pub(super) fn create_default_resources(
    wgpu: Res<WgpuContext>,
    config: Res<RenderConfig>,
    mut commands: Commands,
    mut staging: ResMut<Staging>,
) {
    let sampler = wgpu.device.create_sampler(&Default::default());

    let atlas = Atlas::new(&wgpu.device, Default::default());

    let font = Font::open(&config.default_font, &wgpu.device, &mut *staging).unwrap_or_else(|e| {
        panic!(
            "Error while loading font: {e}: {}",
            config.default_font.display()
        )
    });

    commands.insert_resource(DefaultSampler(sampler));
    commands.insert_resource(DefaultAtlas(atlas));
    commands.insert_resource(DefaultFont(font));
}

// todo: make this a resource that contains all the samplers we use
#[derive(Clone, Debug, Resource)]
pub struct DefaultSampler(pub wgpu::Sampler);

#[derive(Debug, Resource, derive_more::Deref, derive_more::DerefMut)]
pub struct DefaultAtlas(pub Atlas);

#[derive(Debug, Resource, derive_more::Deref, derive_more::DerefMut)]
pub struct DefaultFont(pub Font);
