use std::{
    num::NonZero,
    ops::Index,
    path::Path,
};

use bevy_ecs::{
    resource::Resource,
    schedule::{
        IntoScheduleConfigs,
        SystemSet,
    },
    system::{
        Commands,
        Res,
        ResMut,
    },
    world::World,
};
use bytemuck::{
    Pod,
    Zeroable,
};
use guillotiere::SimpleAtlasAllocator;
use image::RgbaImage;
use nalgebra::{
    Point2,
    Vector2,
};

use crate::{
    ecs::{
        plugin::{
            Plugin,
            WorldBuilder,
        },
        schedule,
    },
    util::image::ImageSizeExt,
    wgpu::{
        WgpuContext,
        WgpuContextBuilder,
        WgpuSystems,
        buffer::{
            WriteStaging,
            WriteStagingBelt,
            WriteStagingCommit,
            WriteStagingTransaction,
        },
        image::{
            ImageTextureExt,
            UnsupportedColorSpace,
        },
    },
};

pub struct AtlasPlugin;

impl Plugin for AtlasPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), color_eyre::eyre::Error> {
        builder
            .add_systems(
                schedule::Startup,
                request_wgpu_features.in_set(WgpuSystems::RequestFeatures),
            )
            .add_systems(
                schedule::Startup,
                create_atlas_builder
                    // we need a wgpu context
                    .after(WgpuSystems::CreateContext)
                    .before(AtlasSystems::CollectTextures),
            )
            .add_systems(
                schedule::Startup,
                create_atlas
                    .in_set(AtlasSystems::BuildAtlas)
                    .after(AtlasSystems::CollectTextures),
            );

        Ok(())
    }
}

fn request_wgpu_features(mut builder: ResMut<WgpuContextBuilder>) {
    // to build the texture atlas we use a texture binding array to bind all images
    // that are put into the atlas at once.
    //
    // todo: technically we could opt out of doing this if the feature is not
    // available. instead we'd issue a separate draw call per image.

    builder
        .request_features(wgpu::Features::TEXTURE_BINDING_ARRAY)
        .request_features(
            wgpu::Features::SAMPLED_TEXTURE_AND_STORAGE_BUFFER_ARRAY_NON_UNIFORM_INDEXING,
        );

    builder
        .enabled_limits
        .max_binding_array_elements_per_shader_stage = builder
        .supported_limits
        .max_binding_array_elements_per_shader_stage;
}

fn create_atlas_builder(wgpu: Res<WgpuContext>, mut commands: Commands) {
    let atlas_builder = AtlasBuilder::new(&wgpu);
    commands.insert_resource(atlas_builder);
}

fn create_atlas(mut commands: Commands) {
    commands.queue(|world: &mut World| {
        let atlas_builder = world.remove_resource::<AtlasBuilder>().unwrap();
        let atlas = atlas_builder.finish();
        world.insert_resource(atlas);
    });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, SystemSet)]
pub enum AtlasSystems {
    CollectTextures,
    BuildAtlas,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Atlas full ({limit}x{limit})")]
    AtlasFull { limit: u32 },

    #[error("Image too large: {}x{} > {limit}x{limit}", .image_size.x, .image_size.y)]
    TooLarge {
        image_size: Vector2<u32>,
        limit: u32,
    },

    #[error(transparent)]
    Image(#[from] image::ImageError),

    #[error(transparent)]
    UnsupportedColorSpace(#[from] UnsupportedColorSpace),
}

#[derive(derive_more::Debug, Resource)]
pub struct AtlasBuilder {
    #[debug(skip)]
    allocator: SimpleAtlasAllocator,
    size: u32,
    size_limit: u32,
    allocations: Vec<AtlasSlot>,
    textures: Vec<wgpu::Texture>,
    device: wgpu::Device,
    queue: wgpu::Queue,

    // can we use the general rendering staging transaction?
    staging: WriteStagingTransaction<WriteStagingBelt, wgpu::Device, wgpu::CommandEncoder>,
}

impl AtlasBuilder {
    pub fn new(wgpu: &WgpuContext) -> Self {
        let initial_size = 512;
        let size_limit = wgpu.info.limits.max_texture_dimension_2d;

        let command_encoder = wgpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("atlas"),
            });
        let staging = WriteStagingTransaction::new(
            wgpu.staging_pool.belt(),
            wgpu.device.clone(),
            command_encoder,
        );

        Self {
            allocator: SimpleAtlasAllocator::new(vector2_to_guillotiere(Vector2::repeat(
                initial_size,
            ))),
            size: initial_size,
            size_limit,
            allocations: vec![],
            textures: vec![],
            device: wgpu.device.clone(),
            queue: wgpu.queue.clone(),
            staging,
        }
    }

    pub fn insert(&mut self, image: &RgbaImage) -> Result<AtlasId, Error> {
        let id = AtlasId(self.allocations.len().try_into().unwrap());
        let image_size = image.size();

        // check if image won't ever fit
        if image_size.x > self.size_limit || image_size.y > self.size_limit {
            return Err(Error::TooLarge {
                image_size,
                limit: self.size_limit,
            });
        }

        // allocate space for image
        let offset = loop {
            if let Some(rectangle) = self.allocator.allocate(vector2_to_guillotiere(image_size)) {
                let min = guillotiere_to_point2(rectangle.min);
                let max = guillotiere_to_point2(rectangle.max);
                assert_eq!(max - min, image_size);

                break min;
            }
            else if self.size < self.size_limit {
                // todo: make sure the new size fits the requested size
                let new_size = (2 * self.size).min(self.size_limit);
                self.allocator
                    .grow(vector2_to_guillotiere(Vector2::repeat(new_size)));
                self.size = new_size;
            }
            else {
                return Err(Error::AtlasFull { limit: self.size });
            }
        };

        // upload image to gpu
        let texture = self
            .staging
            .device
            .create_texture(&image.texture_descriptor(
                "staged image for atlas",
                wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                const { NonZero::new(1).unwrap() },
            )?);
        image.write_to_texture(&texture, &mut self.staging);

        // store allocation
        self.allocations.push(AtlasSlot {
            offset: offset.cast(),
            size: image_size.cast(),
        });
        self.textures.push(texture);

        Ok(id)
    }

    pub fn finish(mut self) -> Atlas {
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;
        let dump_texture_to = Some(Path::new("tmp/texture_atlas.png"));

        // scale slot offsets and sizes to final atlas size
        let atlas_size_inv = 1.0 / self.size as f32;
        for slot in &mut self.allocations {
            slot.offset *= atlas_size_inv;
            slot.size *= atlas_size_inv;
        }

        // create atlas texture
        let atlas_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("atlas"),
            size: wgpu::Extent3d {
                width: self.size,
                height: self.size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        let atlas_texture_view = atlas_texture.create_view(&wgpu::TextureViewDescriptor {
            label: Some("atlas"),
            ..Default::default()
        });

        // create atlas data buffer
        let atlas_data_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("atlas data"),
            size: (size_of::<AtlasDataEntry>() * self.allocations.len().max(1))
                as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: true,
        });

        {
            let mut buffer_view = self.staging.write_buffer(atlas_data_buffer.slice(..));
            let buffer_view = bytemuck::cast_slice_mut::<u8, AtlasDataEntry>(&mut *buffer_view);

            for (buffer_entry, allocation) in buffer_view.iter_mut().zip(self.allocations.iter()) {
                buffer_entry.offset = allocation.offset;
                buffer_entry.size = allocation.size;
            }

            atlas_data_buffer.unmap();
        }

        // create atlas sampler
        let atlas_sampler = self
            .staging
            .device
            .create_sampler(&wgpu::SamplerDescriptor {
                label: Some("atlas blit"),
                address_mode_u: wgpu::AddressMode::Repeat,
                address_mode_v: wgpu::AddressMode::Repeat,
                address_mode_w: wgpu::AddressMode::Repeat,
                mag_filter: wgpu::FilterMode::Nearest,
                min_filter: wgpu::FilterMode::Nearest,
                ..Default::default()
            });

        // blit images into atlas texture
        if !self.allocations.is_empty() {
            // create pipeline
            let blit_bind_group_layout =
                self.device
                    .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                        label: Some("atlas blit"),
                        entries: &[
                            wgpu::BindGroupLayoutEntry {
                                binding: 0,
                                visibility: wgpu::ShaderStages::FRAGMENT,
                                ty: wgpu::BindingType::Texture {
                                    sample_type: wgpu::TextureSampleType::Float {
                                        filterable: true,
                                    },
                                    view_dimension: wgpu::TextureViewDimension::D2,
                                    multisampled: false,
                                },
                                count: Some(
                                    NonZero::new(u32::try_from(self.allocations.len()).unwrap())
                                        .unwrap(),
                                ),
                            },
                            wgpu::BindGroupLayoutEntry {
                                binding: 1,
                                visibility: wgpu::ShaderStages::FRAGMENT,
                                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                                count: None,
                            },
                            wgpu::BindGroupLayoutEntry {
                                binding: 2,
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

            let pipeline_layout =
                self.device
                    .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("atlas blit"),
                        bind_group_layouts: &[&blit_bind_group_layout],
                        immediate_size: 0,
                    });

            let module = self
                .device
                .create_shader_module(wgpu::include_wgsl!("texture_atlas.wgsl"));

            let pipeline = self
                .device
                .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("atlas blit"),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &module,
                        entry_point: Some("blit_vertex"),
                        compilation_options: Default::default(),
                        buffers: &[],
                    },
                    primitive: wgpu::PrimitiveState {
                        topology: wgpu::PrimitiveTopology::TriangleStrip,
                        strip_index_format: None,
                        front_face: wgpu::FrontFace::Ccw,
                        cull_mode: None,
                        unclipped_depth: false,
                        polygon_mode: wgpu::PolygonMode::Fill,
                        conservative: false,
                    },
                    depth_stencil: None,
                    multisample: Default::default(),
                    fragment: Some(wgpu::FragmentState {
                        module: &module,
                        entry_point: Some("blit_fragment"),
                        compilation_options: Default::default(),
                        targets: &[Some(wgpu::ColorTargetState {
                            format,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                    }),
                    multiview_mask: None,
                    cache: None,
                });

            // create blit bind group
            let blit_bind_group = {
                let texture_views = self
                    .textures
                    .iter()
                    .map(|texture| {
                        texture.create_view(&wgpu::TextureViewDescriptor {
                            label: Some("atlas blit source"),
                            ..Default::default()
                        })
                    })
                    .collect::<Vec<_>>();
                let texture_views = texture_views.iter().collect::<Vec<_>>();

                self.staging
                    .device
                    .create_bind_group(&wgpu::BindGroupDescriptor {
                        label: Some("atlas blit"),
                        layout: &blit_bind_group_layout,
                        entries: &[
                            wgpu::BindGroupEntry {
                                binding: 0,
                                resource: wgpu::BindingResource::TextureViewArray(&texture_views),
                            },
                            wgpu::BindGroupEntry {
                                binding: 1,
                                resource: wgpu::BindingResource::Sampler(&atlas_sampler),
                            },
                            wgpu::BindGroupEntry {
                                binding: 2,
                                resource: atlas_data_buffer.as_entire_binding(),
                            },
                        ],
                    })
            };

            // blit
            {
                let mut render_pass =
                    self.staging
                        .command_encoder
                        .begin_render_pass(&wgpu::RenderPassDescriptor {
                            label: Some("atlas blit"),
                            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                view: &atlas_texture_view,
                                depth_slice: None,
                                resolve_target: None,
                                ops: wgpu::Operations {
                                    load: wgpu::LoadOp::Clear(wgpu::Color {
                                        r: 1.0,
                                        g: 0.0,
                                        b: 1.0,
                                        a: 1.0,
                                    }),
                                    store: wgpu::StoreOp::Store,
                                },
                            })],
                            depth_stencil_attachment: None,
                            timestamp_writes: None,
                            occlusion_query_set: None,
                            multiview_mask: None,
                        });

                let num_instances = u32::try_from(self.allocations.len()).unwrap();
                render_pass.set_pipeline(&pipeline);
                render_pass.set_bind_group(0, Some(&blit_bind_group), &[]);
                render_pass.draw(0..4, 0..num_instances);
            }
        }

        // dump atlas texture (for debugging)
        if let Some(path) = dump_texture_to {
            tracing::debug!(path = %path.display(), "dumping texture atlas");

            let staging_buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("atlas read-back staging"),
                size: (self.size * self.size * 4) as wgpu::BufferAddress,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });

            self.staging.command_encoder.copy_texture_to_buffer(
                wgpu::TexelCopyTextureInfo {
                    texture: &atlas_texture,
                    mip_level: 0,
                    origin: Default::default(),
                    aspect: Default::default(),
                },
                wgpu::TexelCopyBufferInfo {
                    buffer: &staging_buffer,
                    layout: wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(self.size * 4),
                        rows_per_image: None,
                    },
                },
                wgpu::Extent3d {
                    width: self.size,
                    height: self.size,
                    depth_or_array_layers: 1,
                },
            );

            self.staging
                .command_encoder
                .on_submitted_work_done(move || {
                    staging_buffer
                        .clone()
                        .map_async(wgpu::MapMode::Read, .., move |result| {
                            result.unwrap();
                            let mapped_range = staging_buffer.get_mapped_range(..);

                            let image = image::ImageBuffer::<image::Rgba<u8>, _>::from_raw(
                                self.size,
                                self.size,
                                &*mapped_range,
                            )
                            .unwrap();

                            if let Err(error) = image.save(path) {
                                tracing::error!(path = %path.display(), "couldn't save texture atlas: {error}");
                            }
                        });
                });
        }

        // commit staging transaction and submit command buffer
        let command_encoder = self.staging.commit();
        let _submission_index = self.queue.submit([command_encoder.finish()]);

        // create atlas bind group
        let atlas_bind_group_layout =
            self.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("mesh"),
                    entries: &[
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                view_dimension: wgpu::TextureViewDimension::D2,
                                multisampled: false,
                            },
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                        wgpu::BindGroupLayoutEntry {
                            binding: 2,
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

        let atlas_bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("atlas"),
            layout: &atlas_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&atlas_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&atlas_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: atlas_data_buffer.as_entire_binding(),
                },
            ],
        });

        Atlas {
            bind_group_layout: atlas_bind_group_layout,
            bind_group: atlas_bind_group,
            allocations: self.allocations,
        }
    }
}

#[derive(Clone, Debug, Resource)]
pub struct Atlas {
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    allocations: Vec<AtlasSlot>,
}

impl Atlas {
    pub fn bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.bind_group_layout
    }

    pub fn bind_group(&self) -> &wgpu::BindGroup {
        &self.bind_group
    }
}

impl Index<AtlasId> for Atlas {
    type Output = AtlasSlot;

    fn index(&self, index: AtlasId) -> &Self::Output {
        &self.allocations[usize::try_from(index.0).unwrap()]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, derive_more::Into)]
pub struct AtlasId(u32);

#[derive(Clone, Debug)]
pub struct AtlasSlot {
    offset: Point2<f32>,
    size: Vector2<f32>,
}

impl AtlasSlot {
    pub fn map_uv(&self, uv: Point2<f32>) -> Point2<f32> {
        self.offset + uv.coords.component_mul(&self.size)
    }
}

fn vector2_to_guillotiere(size: Vector2<u32>) -> guillotiere::Size {
    guillotiere::Size::new(
        i32::try_from(size.x).unwrap(),
        i32::try_from(size.y).unwrap(),
    )
}

fn guillotiere_to_point2(point: guillotiere::Point) -> Point2<u32> {
    Point2::new(point.x.try_into().unwrap(), point.y.try_into().unwrap())
}

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
struct AtlasDataEntry {
    offset: Point2<f32>,
    size: Vector2<f32>,
}
