use bevy_ecs::{
    change_detection::DetectChanges,
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
use nalgebra::Vector2;

use crate::{
    app::Time,
    render::{
        DefaultAtlas,
        DefaultSampler,
        atlas::AtlasResources,
        camera::{
            Camera,
            CameraData,
        },
        pass::RenderPass,
        render_target::RenderTarget,
        staging::Staging,
        surface::{
            ClearColor,
            Surface,
        },
    },
    wgpu::{
        WgpuContext,
        buffer::WriteStaging,
        srgba_to_wgpu,
    },
};

#[derive(Debug, Component)]
pub struct MainPass {
    active: Option<ActiveMainPass>,
    bind_group: wgpu::BindGroup,
}

#[derive(Debug)]
struct ActiveMainPass {
    command_encoder: wgpu::CommandEncoder,
    render_pass: RenderPass<'static>,
    /// todo: make it possible to render to normal textures as well
    surface_texture: wgpu::SurfaceTexture,
}

#[derive(Debug, Resource)]
pub struct MainPassLayout {
    pub bind_group_layout: wgpu::BindGroupLayout,
}

#[derive(Debug, Component)]
pub struct MainPassUniform {
    buffer: wgpu::Buffer,
    pub data: MainPassUniformData,
}

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
pub struct MainPassUniformData {
    pub camera: CameraData,
    pub time: f32,
    _padding: [u32; 3],
}

#[profiling::function]
pub fn create_layout(wgpu: Res<WgpuContext>, mut commands: Commands) {
    let bind_group_layout =
        wgpu.device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("main pass"),
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
                ],
            });

    commands.insert_resource(MainPassLayout { bind_group_layout });
}

#[profiling::function]
pub fn update_uniform(
    frame_uniforms: Populated<&mut MainPassUniform>,
    mut staging: ResMut<Staging>,
    time: Res<Time>,
) {
    for mut frame_uniform in frame_uniforms {
        frame_uniform.data.time = time.tick_start_seconds();

        // update frame uniform buffer
        staging.write_buffer_from_slice(
            frame_uniform.buffer.slice(..),
            bytemuck::bytes_of(&frame_uniform.data),
        );
    }
}

#[profiling::function]
pub fn update_bind_group(
    wgpu: Res<WgpuContext>,
    main_passes: Query<(&mut MainPass, &MainPassUniform)>,
    mut atlas: ResMut<DefaultAtlas>,
    default_sampler: Res<DefaultSampler>,
    mut staging: ResMut<Staging>,
    frame_bind_group_layout: Res<MainPassLayout>,
) {
    // todo: separate the atlas flushing into its own system, since multiple passes
    // might use the atlas
    if atlas.0.flush(&wgpu.device, &mut *staging) {
        let atlas_resources = atlas.0.resources();

        for (mut main_pass, main_pass_uniform) in main_passes {
            // recreate the bind group
            main_pass.bind_group = create_bind_group(
                &wgpu.device,
                &frame_bind_group_layout,
                main_pass_uniform,
                &default_sampler,
                atlas_resources,
            )
        }
    }
}

#[profiling::function]
pub fn create_main_pass(
    wgpu: Res<WgpuContext>,
    main_pass_layout: Res<MainPassLayout>,
    cameras: Populated<Entity, (With<Camera>, Without<MainPass>)>,
    default_sampler: Res<DefaultSampler>,
    default_atlas: Res<DefaultAtlas>,
    mut commands: Commands,
) {
    for entity in cameras {
        let main_pass_uniform = {
            let buffer = wgpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("frame uniform"),
                size: size_of::<MainPassUniformData>() as wgpu::BufferAddress,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
                mapped_at_creation: false,
            });

            MainPassUniform {
                buffer,
                data: Zeroable::zeroed(),
            }
        };

        let bind_group = create_bind_group(
            &wgpu.device,
            &main_pass_layout,
            &main_pass_uniform,
            &default_sampler,
            default_atlas.0.resources(),
        );

        let mut entity = commands.entity(entity);
        entity.insert((
            MainPass {
                active: None,
                bind_group,
            },
            main_pass_uniform,
        ));
    }
}

#[profiling::function]
pub fn begin_pass(
    wgpu: Res<WgpuContext>,
    cameras: Populated<(NameOrEntity, &RenderTarget, &mut MainPass), With<Camera>>,
    surfaces: Populated<(&Surface, Option<&ClearColor>)>,
    mut commands: Commands,
) {
    for (entity, render_target, mut main_pass) in cameras {
        assert!(
            main_pass.active.is_none(),
            "A main pass is still active for `{}`",
            entity
        );

        if let Ok((surface, clear_color)) = surfaces.get(render_target.0) {
            let mut command_encoder =
                wgpu.device
                    .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                        label: Some("main pass"),
                    });

            let surface_texture = surface.surface_texture();
            let surface_texture_view =
                surface_texture
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor {
                        label: Some("main pass render target"),
                        ..Default::default()
                    });

            let mut profiler = wgpu
                .profiler
                .as_ref()
                .map(|profiler| profiler.begin_render_pass("frame"));

            let mut render_pass = command_encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("main pass"),
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
                    timestamp_writes: profiler
                        .as_mut()
                        .map(|transaction| transaction.timestamp_writes()),
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();

            // bind frame uniform buffer
            render_pass.set_bind_group(0, Some(&main_pass.bind_group), &[]);

            main_pass.active = Some(ActiveMainPass {
                command_encoder,
                render_pass: RenderPass {
                    render_pass,
                    profiler,
                },
                surface_texture,
            });
        }
        else {
            panic!("No surface on render target")
        }
    }
}

#[profiling::function]
pub fn end_pass(
    wgpu: Res<WgpuContext>,
    main_passes: Populated<&mut MainPass>,
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
    for mut main_pass in main_passes {
        if let Some(ActiveMainPass {
            mut command_encoder,
            render_pass,
            surface_texture,
        }) = main_pass.active.take()
        {
            // drop the render pass explicitely since we'll submit the command encoder next
            drop(render_pass.render_pass);

            if let Some(profiler) = render_pass.profiler {
                profiler.finish(&mut command_encoder);
            }

            // finish the frame's renderpass command encoder
            command_buffers.push(command_encoder.finish());

            // and present after we submit
            present_surfaces.push(surface_texture);
        }
    }

    // submit all command buffers
    wgpu.queue.submit(command_buffers.drain(..));

    // present surfaces
    for surface_texture in present_surfaces.drain(..) {
        surface_texture.present();
    }
}

fn create_bind_group(
    device: &wgpu::Device,
    main_pass_layout: &MainPassLayout,
    main_pass_uniform: &MainPassUniform,
    default_sampler: &DefaultSampler,
    atlas_resources: AtlasResources,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("main pass bind group"),
        layout: &main_pass_layout.bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: main_pass_uniform.buffer.as_entire_binding(),
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
        ],
    })
}
