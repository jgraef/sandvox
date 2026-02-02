use bevy_ecs::{
    component::Component,
    entity::Entity,
    name::NameOrEntity,
    query::{
        Has,
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
        ParamSet,
        Populated,
        Query,
        Res,
        ResMut,
        SystemParam,
    },
};
use bytemuck::{
    Pod,
    Zeroable,
};
use color_eyre::eyre::Error;

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
        DefaultSampler,
        RenderFunctions,
        RenderPlugin,
        RenderSystems,
        atlas::AtlasResources,
        camera::{
            Camera,
            CameraData,
        },
        pass::{
            context::RenderContext,
            phase,
        },
        render_target::RenderTarget,
        staging::Staging,
        surface::Surface,
    },
    wgpu::{
        WgpuContext,
        buffer::WriteStaging,
    },
};

#[derive(Clone, Copy, Debug, Default)]
pub struct MainPassPlugin;

impl Plugin for MainPassPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder
            .require_plugin::<RenderPlugin>()
            .add_systems(
                schedule::Startup,
                (
                    (create_layout, create_main_pass).chain(),
                    update_main_pass_uniform,
                )
                    .in_set(MainPassSystems::Prepare),
            )
            .add_systems(
                schedule::Render,
                (
                    (create_layout, create_main_pass)
                        .chain()
                        .in_set(MainPassSystems::Prepare),
                    render_main_pass.in_set(MainPassSystems::Render),
                    (
                        update_main_pass_uniform,
                        update_main_pass.run_if(resource_changed::<DefaultAtlas>),
                    )
                        .in_set(RenderSystems::EndFrame),
                ),
            )
            .configure_system_sets(
                schedule::Startup,
                MainPassSystems::Prepare.in_set(RenderSystems::Setup),
            )
            .configure_system_sets(
                schedule::Render,
                MainPassSystems::Prepare.in_set(RenderSystems::BeginFrame),
            )
            .configure_system_sets(
                schedule::Render,
                MainPassSystems::Render
                    .in_set(RenderSystems::Render)
                    .after(MainPassSystems::Prepare)
                    .before(RenderSystems::EndFrame),
            );

        Ok(())
    }
}

#[derive(Clone, Copy, Debug, SystemSet, PartialEq, Eq, Hash)]
pub enum MainPassSystems {
    Prepare,
    Render,
}

#[derive(Debug, Component)]
pub struct MainPass {
    bind_group: wgpu::BindGroup,
}

/// Attach to camera to enable depth prepass
///
/// This must be attached when the camera is created as e.g. the mesh pipeline
/// only initializes the depth prepass pipeline then.
#[derive(Debug, Component)]
pub struct DepthPrepass;

/// Attach to camera to render wireframes
#[derive(Debug, Component)]
pub struct Wireframe;

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
fn create_layout(wgpu: Res<WgpuContext>, mut commands: Commands) {
    let bind_group_layout =
        wgpu.device
            .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("main pass"),
                entries: &[
                    // uniform. contains camera matrix, etc.
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
fn update_main_pass_uniform(
    uniforms: Populated<&mut MainPassUniform>,
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
fn update_main_pass(
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
fn create_main_pass(
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
        entity.insert((MainPass { bind_group }, main_pass_uniform));
    }
}

#[derive(derive_more::Debug, SystemParam)]
struct MainPassRenderFunctions<'w, 's> {
    #[debug(skip)]
    set: ParamSet<
        'w,
        's,
        (
            RenderFunctions<'w, 's, phase::Opaque>,
            RenderFunctions<'w, 's, phase::DepthPrepass>,
            RenderFunctions<'w, 's, phase::Wireframe>,
            RenderFunctions<'w, 's, phase::Skybox>,
        ),
    >,
}

impl<'w, 's> MainPassRenderFunctions<'w, 's> {
    fn opaque(&mut self) -> RenderFunctions<'_, '_, phase::Opaque> {
        self.set.p0()
    }

    fn depth_prepass(&mut self) -> RenderFunctions<'_, '_, phase::DepthPrepass> {
        self.set.p1()
    }

    fn wireframe(&mut self) -> RenderFunctions<'_, '_, phase::Wireframe> {
        self.set.p2()
    }

    fn skybox(&mut self) -> RenderFunctions<'_, '_, phase::Skybox> {
        self.set.p3()
    }
}

#[profiling::function]
fn render_main_pass(
    mut render_context: RenderContext,
    cameras: Populated<
        (
            NameOrEntity,
            &RenderTarget,
            &MainPass,
            Has<Wireframe>,
            Has<DepthPrepass>,
        ),
        With<Camera>,
    >,
    surfaces: Populated<&Surface>,
    mut render_functions: MainPassRenderFunctions,
    any_wireframe: Query<(), (With<MainPass>, With<Wireframe>)>,
    any_depth_prepass: Query<(), (With<MainPass>, With<Wireframe>)>,
) {
    let any_wireframe = any_wireframe.is_empty();
    let any_depth_prepass = any_depth_prepass.is_empty();

    // prepare
    render_functions.opaque().prepare();
    if any_depth_prepass {
        render_functions.depth_prepass().prepare();
    }
    if any_wireframe {
        render_functions.wireframe().prepare();
    }

    render_functions.skybox().prepare();

    for (camera_entity, render_target, main_pass, wireframe, depth_prepass) in cameras {
        // get target texture (and clear color)
        // todo: this should work with any kind of target texture
        let surface = surfaces.get(render_target.0).unwrap();

        if depth_prepass {
            assert!(any_depth_prepass);

            run_z_prepass_on_surface(
                &mut render_context,
                &mut render_functions,
                surface,
                main_pass,
                camera_entity.entity,
            );
        }

        // !any_wireframe => !wireframe
        assert!(any_wireframe || !wireframe);

        run_main_pass_on_surface(
            &mut render_context,
            &mut render_functions,
            surface,
            main_pass,
            camera_entity.entity,
            wireframe,
            depth_prepass,
        );
    }
}

#[profiling::function]
fn run_z_prepass_on_surface(
    render_context: &mut RenderContext,
    render_functions: &mut MainPassRenderFunctions,
    surface: &Surface,
    main_pass: &MainPass,
    camera_entity: Entity,
) {
    let depth_texture_view = surface.depth_texture();

    // create render pass
    let mut render_pass = render_context.begin_render_pass(
        &wgpu::RenderPassDescriptor {
            label: Some("z-prepass"),
            color_attachments: &[],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_texture_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        },
        "z-prepass",
    );

    // bind frame uniform buffer
    render_pass.set_bind_group(0, Some(&main_pass.bind_group), &[]);

    // render!
    render_functions
        .depth_prepass()
        .render(&mut render_pass, camera_entity);
}

#[profiling::function]
fn run_main_pass_on_surface(
    render_context: &mut RenderContext,
    render_functions: &mut MainPassRenderFunctions,
    surface: &Surface,
    main_pass: &MainPass,
    camera_entity: Entity,
    wireframe: bool,
    depth_prepass: bool,
) {
    let surface_texture_view = surface.surface_texture();
    let depth_texture_view = surface.depth_texture();

    // create render pass
    let mut render_pass = render_context.begin_render_pass(
        &wgpu::RenderPassDescriptor {
            label: Some("main pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &surface_texture_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Load,
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_texture_view,
                depth_ops: Some(if depth_prepass {
                    wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Discard,
                    }
                }
                else {
                    wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        },
        "main_pass",
    );

    // bind frame uniform buffer
    render_pass.set_bind_group(0, Some(&main_pass.bind_group), &[]);

    // render!
    render_functions
        .opaque()
        .render(&mut render_pass, camera_entity);

    if wireframe {
        render_functions
            .wireframe()
            .render(&mut render_pass, camera_entity);
    }

    render_functions
        .skybox()
        .render(&mut render_pass, camera_entity);
}

#[profiling::function]
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
