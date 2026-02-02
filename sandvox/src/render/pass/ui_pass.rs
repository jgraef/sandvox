use bevy_ecs::{
    component::Component,
    entity::Entity,
    name::NameOrEntity,
    query::{
        With,
        Without,
    },
    resource::Resource,
    schedule::{
        IntoScheduleConfigs,
        SystemSet,
        common_conditions::resource_changed,
    },
    system::{
        Commands,
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
use color_eyre::eyre::Error;
use nalgebra::Vector2;

use crate::{
    app::Time,
    ecs::{
        plugin::{
            Plugin,
            WorldBuilder,
        },
        schedule,
    },
    render::{
        DefaultAtlas,
        DefaultFont,
        DefaultSampler,
        RenderFunctions,
        RenderPlugin,
        RenderSystems,
        atlas::AtlasResources,
        pass::{
            context::RenderContext,
            phase,
        },
        render_target::RenderTarget,
        staging::Staging,
        surface::{
            ClearColor,
            Surface,
        },
        text::FontResources,
    },
    ui,
    wgpu::{
        WgpuContext,
        buffer::WriteStaging,
        srgba_to_wgpu,
    },
};

#[derive(Clone, Copy, Debug, Default)]
pub struct UiPassPlugin;

impl Plugin for UiPassPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder
            .require_plugin::<RenderPlugin>()
            .add_systems(
                schedule::Startup,
                (
                    (create_layout, create_ui_pass).chain(),
                    update_ui_pass_uniform,
                )
                    .in_set(UiPassSystems::Prepare),
            )
            .add_systems(
                schedule::Render,
                (
                    (create_layout, create_ui_pass)
                        .chain()
                        .in_set(UiPassSystems::Prepare),
                    render_ui_pass.in_set(UiPassSystems::Render),
                    (
                        update_ui_pass_uniform,
                        update_ui_pass.run_if(resource_changed::<DefaultAtlas>),
                    )
                        .in_set(RenderSystems::EndFrame),
                ),
            )
            .configure_system_sets(
                schedule::Startup,
                UiPassSystems::Prepare.in_set(RenderSystems::Setup),
            )
            .configure_system_sets(
                schedule::Render,
                UiPassSystems::Prepare.in_set(RenderSystems::BeginFrame),
            )
            .configure_system_sets(
                schedule::Render,
                UiPassSystems::Render
                    .in_set(RenderSystems::Render)
                    .after(UiPassSystems::Prepare)
                    .before(RenderSystems::EndFrame),
            );

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, SystemSet, PartialEq, Eq, Hash)]
pub enum UiPassSystems {
    Prepare,
    Render,
}

#[derive(Debug, Component)]
pub struct UiPass {
    bind_group: wgpu::BindGroup,
}

#[derive(Debug, Resource)]
pub struct UiPassLayout {
    pub bind_group_layout: wgpu::BindGroupLayout,
}

#[derive(Debug, Component)]
pub struct UiPassUniform {
    buffer: wgpu::Buffer,
    pub data: UiPassUniformData,
}

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
pub struct UiPassUniformData {
    pub viewport_size: Vector2<u32>,
    pub time: f32,
    _padding: u32,
}

#[profiling::function]
fn create_layout(wgpu: Res<WgpuContext>, mut commands: Commands) {
    let bind_group_layout =
        wgpu.device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("ui pass"),
                entries: &[
                    // uniform. contains viewport size
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

    commands.insert_resource(UiPassLayout { bind_group_layout });
}

#[profiling::function]
fn update_ui_pass_uniform(
    uniforms: Populated<&mut UiPassUniform>,
    mut staging: ResMut<Staging>,
    time: Res<Time>,
) {
    for mut uniform in uniforms {
        uniform.data.time = time.tick_start_seconds();

        // update frame uniform buffer
        staging
            .write_buffer_from_slice(uniform.buffer.slice(..), bytemuck::bytes_of(&uniform.data));
    }
}

#[profiling::function]
fn create_ui_pass(
    wgpu: Res<WgpuContext>,
    ui_pass_layout: Res<UiPassLayout>,
    views: Populated<Entity, (With<ui::View>, Without<UiPass>)>,
    default_sampler: Res<DefaultSampler>,
    default_atlas: Res<DefaultAtlas>,
    default_font: Res<DefaultFont>,
    mut commands: Commands,
) {
    for entity in views {
        let ui_pass_uniform = {
            let buffer = wgpu.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("frame uniform"),
                size: size_of::<UiPassUniformData>() as wgpu::BufferAddress,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::UNIFORM,
                mapped_at_creation: false,
            });

            UiPassUniform {
                buffer,
                data: Zeroable::zeroed(),
            }
        };

        let bind_group = create_bind_group(
            &wgpu.device,
            &ui_pass_layout,
            &ui_pass_uniform,
            &default_sampler,
            default_atlas.0.resources(),
            default_font.0.resources(),
        );

        let mut entity = commands.entity(entity);
        entity.insert((UiPass { bind_group }, ui_pass_uniform));
    }
}

#[profiling::function]
fn update_ui_pass(
    wgpu: Res<WgpuContext>,
    ui_passes: Query<(&mut UiPass, &UiPassUniform)>,
    mut default_atlas: ResMut<DefaultAtlas>,
    default_font: Res<DefaultFont>,
    default_sampler: Res<DefaultSampler>,
    mut staging: ResMut<Staging>,
    frame_bind_group_layout: Res<UiPassLayout>,
) {
    // todo: separate the atlas flushing into its own system, since multiple passes
    // might use the atlas
    if default_atlas.0.flush(&wgpu.device, &mut *staging) {
        let atlas_resources = default_atlas.0.resources();
        let font_resources = default_font.0.resources();

        for (mut ui_pass, ui_pass_uniform) in ui_passes {
            // recreate the bind group
            ui_pass.bind_group = create_bind_group(
                &wgpu.device,
                &frame_bind_group_layout,
                ui_pass_uniform,
                &default_sampler,
                atlas_resources,
                font_resources,
            )
        }
    }
}

#[profiling::function]
fn render_ui_pass(
    mut render_context: RenderContext,
    views: Populated<(NameOrEntity, &RenderTarget, &UiPass, Option<&ClearColor>), With<ui::View>>,
    surfaces: Populated<&Surface>,
    mut render_functions: RenderFunctions<phase::Ui>,
) {
    render_functions.prepare();

    for (camera_entity, render_target, ui_pass, clear_color) in views {
        // get target texture (and clear color)
        // todo: this should work with any kind of target texture
        let surface = surfaces.get(render_target.0).unwrap();
        let surface_texture_view = surface.surface_texture();

        let profiler = {
            // create render pass
            let mut render_pass = render_context.begin_render_pass(
                &wgpu::RenderPassDescriptor {
                    label: Some("ui pass"),
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
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                },
                "ui pass",
            );

            // bind frame uniform buffer
            render_pass.set_bind_group(0, Some(&ui_pass.bind_group), &[]);

            // render!
            render_functions.render(&mut render_pass, camera_entity.entity);

            render_pass.profiler
            // actual render pass dropped here
        };

        if let Some(profiler) = profiler {
            profiler.finish(render_context.command_encoder());
        }
    }
}

#[profiling::function]
fn create_bind_group(
    device: &wgpu::Device,
    ui_pass_layout: &UiPassLayout,
    ui_pass_uniform: &UiPassUniform,
    default_sampler: &DefaultSampler,
    atlas_resources: AtlasResources,
    font_resources: FontResources,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("ui pass bind group"),
        layout: &ui_pass_layout.bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: ui_pass_uniform.buffer.as_entire_binding(),
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
    })
}
