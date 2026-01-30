use std::collections::HashMap;

use bytemuck::{
    Pod,
    Zeroable,
};
use indexmap::IndexSet;
use nalgebra::{
    Point2,
    Vector2,
};
use palette::LinSrgba;

use crate::{
    render::staging::Staging,
    wgpu::buffer::TypedArrayBuffer,
};

#[derive(Debug)]
pub struct Blitter {
    blit_bind_group_layout: wgpu::BindGroupLayout,
    blit_pipeline: wgpu::RenderPipeline,
    blit_data_buffer: TypedArrayBuffer<BlitData>,
    fill_bind_group_layout: wgpu::BindGroupLayout,
    fill_pipeline: wgpu::RenderPipeline,
    fill_data_buffer: TypedArrayBuffer<FillData>,
    transaction_workspace: BlitterTransactionWorkspace,
}

impl Blitter {
    pub fn new(device: &wgpu::Device) -> Self {
        let format = wgpu::TextureFormat::Rgba8UnormSrgb;

        let blit_shader = device.create_shader_module(wgpu::include_wgsl!("blit.wgsl"));

        let blit_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("blit"),
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

        let blit_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("blit"),
            bind_group_layouts: &[&blit_bind_group_layout],
            immediate_size: 0,
        });

        let blit_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blit"),
            layout: Some(&blit_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &blit_shader,
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
                module: &blit_shader,
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

        let fill_shader = device.create_shader_module(wgpu::include_wgsl!("fill.wgsl"));

        let fill_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("blit/fill"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Storage { read_only: true },
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let fill_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("blit/fill"),
            bind_group_layouts: &[&fill_bind_group_layout],
            immediate_size: 0,
        });

        let fill_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("blit/fill"),
            layout: Some(&fill_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &fill_shader,
                entry_point: Some("fill_vertex"),
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
                module: &fill_shader,
                entry_point: Some("fill_fragment"),
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

        let blit_data_buffer = TypedArrayBuffer::new(
            device.clone(),
            "blit",
            wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::STORAGE,
        );

        let fill_data_buffer = TypedArrayBuffer::new(
            device.clone(),
            "blit/fill",
            wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::STORAGE,
        );

        Self {
            blit_bind_group_layout,
            blit_pipeline,
            blit_data_buffer,
            fill_data_buffer,
            fill_bind_group_layout,
            fill_pipeline,
            transaction_workspace: Default::default(),
        }
    }

    pub fn begin<'a>(
        &'a mut self,
        target_texture: &'a wgpu::TextureView,
    ) -> BlitterTransaction<'a> {
        assert!(self.transaction_workspace.blits.is_empty());
        assert!(self.transaction_workspace.fills.is_empty());
        assert!(self.transaction_workspace.source_textures.is_empty());
        assert!(self.transaction_workspace.source_samplers.is_empty());

        let target_texture_size = target_texture.texture().size();

        BlitterTransaction {
            blitter: self,
            inv_target_size: Vector2::new(
                1.0 / target_texture_size.width as f32,
                1.0 / target_texture_size.height as f32,
            ),
            num_blits: 0,
            target_texture,
        }
    }
}

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
struct BlitData {
    source_offset: Point2<f32>,
    source_size: Vector2<f32>,
    target_offset: Point2<f32>,
    target_size: Vector2<f32>,
}

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
struct FillData {
    color: LinSrgba<f32>,
    target_offset: Point2<f32>,
    target_size: Vector2<f32>,
}

#[derive(Debug)]
pub struct BlitterTransaction<'a> {
    blitter: &'a mut Blitter,
    inv_target_size: Vector2<f32>,
    num_blits: usize,
    target_texture: &'a wgpu::TextureView,
}

impl<'a> BlitterTransaction<'a> {
    pub fn blit(
        &mut self,
        source_texture: &wgpu::TextureView,
        source_sampler: &wgpu::Sampler,
        source_offset: Point2<i32>,
        source_size: Vector2<u32>,
        target_offset: Point2<i32>,
        target_size: Vector2<u32>,
    ) {
        let source_texture_size = source_texture.texture().size();
        let source_texture_size =
            Vector2::new(source_texture_size.width, source_texture_size.height);
        let inv_source_texture_size = source_texture_size.map(|x| 1.0 / (x as f32));

        let blit_data = BlitData {
            source_offset: source_offset
                .coords
                .cast::<f32>()
                .component_mul(&inv_source_texture_size)
                .into(),
            source_size: source_size
                .cast::<f32>()
                .component_mul(&inv_source_texture_size),
            target_offset: target_offset
                .coords
                .cast::<f32>()
                .component_mul(&self.inv_target_size)
                .into(),
            target_size: target_size
                .cast::<f32>()
                .component_mul(&self.inv_target_size),
        };

        self.blitter
            .transaction_workspace
            .push_blit(source_texture, source_sampler, blit_data);

        self.num_blits += 1;
    }

    pub fn fill(
        &mut self,
        color: LinSrgba<f32>,
        target_offset: Point2<i32>,
        target_size: Vector2<u32>,
    ) {
        let fill_data = FillData {
            color,
            target_offset: target_offset
                .coords
                .cast::<f32>()
                .component_mul(&self.inv_target_size)
                .into(),
            target_size: target_size
                .cast::<f32>()
                .component_mul(&self.inv_target_size),
        };

        self.blitter.transaction_workspace.push_fill(fill_data);
    }

    #[profiling::function]
    pub fn finish(self, device: &wgpu::Device, mut staging: &mut Staging) {
        // todo: use gpu profiler

        let any_blits = self.num_blits > 0;
        let any_fills = !self.blitter.transaction_workspace.fills.is_empty();

        // early exit if there aren't any blits or fills
        if !any_blits && !any_fills {
            return;
        }

        // update fills buffer
        if any_fills {
            self.blitter.fill_data_buffer.write_all(
                &self.blitter.transaction_workspace.fills,
                |_| {},
                &mut staging,
            );
        }

        // update blits buffer
        if any_blits {
            let min_storage_buffer_offset_alignment =
                device.limits().min_storage_buffer_offset_alignment as usize;

            assert!(min_storage_buffer_offset_alignment % size_of::<BlitData>() == 0);

            let buffer_size = {
                let mut offset = 0;
                for (_, blit_set) in &mut self.blitter.transaction_workspace.blits {
                    assert!(!blit_set.data.is_empty());

                    offset = wgpu::util::align_to(offset, min_storage_buffer_offset_alignment);

                    blit_set.buffer_offset = offset;
                    blit_set.buffer_size = blit_set.data.len() * size_of::<BlitData>();

                    offset += blit_set.buffer_size;
                }

                offset / size_of::<BlitData>()
            };

            self.blitter.blit_data_buffer.write_all_with(
                buffer_size,
                |destination: &mut [BlitData]| {
                    for (_, blit_set) in &mut self.blitter.transaction_workspace.blits {
                        destination[blit_set.buffer_offset / size_of::<BlitData>()..]
                            [..blit_set.buffer_size / size_of::<BlitData>()]
                            .copy_from_slice(&blit_set.data);
                    }
                },
                |_new_buffer| {},
                &mut staging,
            );
        }

        // create render pass (one for all blits and fills)
        let mut render_pass =
            staging
                .command_encoder_mut()
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("blit"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: self.target_texture,
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

        // perform fills
        if any_fills {
            render_pass.set_pipeline(&self.blitter.fill_pipeline);

            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("blit/fill"),
                layout: &self.blitter.fill_bind_group_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: self.blitter.fill_data_buffer.buffer().as_entire_binding(),
                }],
            });

            render_pass.set_bind_group(0, Some(&bind_group), &[]);
            let num_fills: u32 = self
                .blitter
                .transaction_workspace
                .fills
                .len()
                .try_into()
                .unwrap();
            render_pass.draw(0..4, 0..num_fills);
        }

        // perform blits
        if any_blits {
            render_pass.set_pipeline(&self.blitter.blit_pipeline);
            let buffer = self.blitter.blit_data_buffer.buffer();

            for (blit_key, blit_set) in &self.blitter.transaction_workspace.blits {
                let source_texture = self
                    .blitter
                    .transaction_workspace
                    .source_textures
                    .get_index(blit_key.source_texture_index)
                    .unwrap();
                let source_sampler = self
                    .blitter
                    .transaction_workspace
                    .source_samplers
                    .get_index(blit_key.source_sampler_index)
                    .unwrap();

                let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("blit"),
                    layout: &self.blitter.blit_bind_group_layout,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(source_texture),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(source_sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding {
                                buffer,
                                offset: blit_set.buffer_offset as wgpu::BufferAddress,
                                size: Some(
                                    wgpu::BufferSize::new(
                                        blit_set.buffer_size as wgpu::BufferAddress,
                                    )
                                    .unwrap(),
                                ),
                            }),
                        },
                    ],
                });

                render_pass.set_bind_group(0, Some(&bind_group), &[]);
                let num_blits: u32 = blit_set.data.len().try_into().unwrap();
                render_pass.draw(0..4, 0..num_blits);
            }

            // recall BlitSet buffers to be reused
            self.blitter.transaction_workspace.blit_buffers.extend(
                self.blitter
                    .transaction_workspace
                    .blits
                    .drain()
                    .map(|(_, mut blit_set)| {
                        blit_set.data.clear();
                        blit_set.data
                    }),
            );
        }
    }
}

#[derive(Debug)]
struct BlitSet {
    data: Vec<BlitData>,
    buffer_offset: usize,
    buffer_size: usize,
}

#[derive(Debug, Default)]
struct BlitterTransactionWorkspace {
    source_textures: IndexSet<wgpu::TextureView>,
    source_samplers: IndexSet<wgpu::Sampler>,
    blits: HashMap<BlitKey, BlitSet>,
    blit_buffers: Vec<Vec<BlitData>>,
    fills: Vec<FillData>,
}

impl BlitterTransactionWorkspace {
    pub fn push_blit(
        &mut self,
        source_texture: &wgpu::TextureView,
        source_sampler: &wgpu::Sampler,
        blit_data: BlitData,
    ) {
        let source_texture_index =
            if let Some(index) = self.source_textures.get_index_of(source_texture) {
                index
            }
            else {
                let (index, _) = self.source_textures.insert_full(source_texture.clone());
                index
            };

        let source_sampler_index =
            if let Some(index) = self.source_samplers.get_index_of(source_sampler) {
                index
            }
            else {
                let (index, _) = self.source_samplers.insert_full(source_sampler.clone());
                index
            };

        let blit_key = BlitKey {
            source_texture_index,
            source_sampler_index,
        };

        if let Some(blits) = self.blits.get_mut(&blit_key) {
            blits.data.push(blit_data);
        }
        else {
            let mut data = self.blit_buffers.pop().unwrap_or_default();
            assert!(data.is_empty());
            data.push(blit_data);

            self.blits.insert(
                blit_key,
                BlitSet {
                    data,
                    buffer_offset: 0,
                    buffer_size: 0,
                },
            );
        }
    }

    pub fn push_fill(&mut self, fill_data: FillData) {
        self.fills.push(fill_data);
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct BlitKey {
    source_texture_index: usize,
    source_sampler_index: usize,
}
