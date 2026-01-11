pub mod buffer;
pub mod image;

use std::{
    num::NonZero,
    sync::Arc,
};

use bevy_ecs::{
    resource::Resource,
    schedule::{
        IntoScheduleConfigs,
        SystemSet,
    },
    system::Commands,
    world::World,
};
use color_eyre::eyre::Error;
use nalgebra::Vector2;
use palette::LinSrgba;
use serde::{
    Deserialize,
    Serialize,
};

use crate::{
    ecs::{
        plugin::{
            Plugin,
            WorldBuilder,
        },
        schedule,
    },
    wgpu::buffer::{
        StagingPool,
        WriteStaging,
    },
};

#[derive(Clone, Copy, Debug, Default)]
pub struct WgpuPlugin {
    pub config: WgpuConfig,
}

impl Plugin for WgpuPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        let context_builder = WgpuContextBuilder::new(self.config)?;
        builder.insert_resource(context_builder).add_systems(
            schedule::Startup,
            create_wgpu_context
                .in_set(WgpuSystems::CreateContext)
                .after(WgpuSystems::RequestFeatures),
        );

        Ok(())
    }
}

fn create_wgpu_context(mut commands: Commands) {
    commands.queue(|world: &mut World| {
        let context_builder = world.remove_resource::<WgpuContextBuilder>().unwrap();
        let context = context_builder.build().unwrap();
        world.insert_resource(context);
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, SystemSet)]
pub enum WgpuSystems {
    CreateContext,
    RequestFeatures,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct WgpuConfig {
    #[serde(with = "crate::util::serde::backends")]
    pub backends: wgpu::Backends,
    #[serde(with = "crate::util::serde::power_preference")]
    pub power_preference: wgpu::PowerPreference,
    pub staging_chunk_size: wgpu::BufferSize,
    pub memory_hints: MemoryHints,
}

impl Default for WgpuConfig {
    fn default() -> Self {
        Self {
            backends: wgpu::Backends::VULKAN,
            power_preference: Default::default(),
            staging_chunk_size: const { wgpu::BufferSize::new(0x4000).unwrap() },
            memory_hints: Default::default(),
        }
    }
}

#[derive(Debug, Resource)]
pub struct WgpuContextBuilder {
    pub config: WgpuConfig,
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub adapter_info: wgpu::AdapterInfo,
    pub supported_features: wgpu::Features,
    pub supported_limits: wgpu::Limits,
    pub enabled_features: wgpu::Features,
    pub enabled_limits: wgpu::Limits,
}

impl WgpuContextBuilder {
    pub fn new(config: WgpuConfig) -> Result<Self, Error> {
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: config.backends,
            ..Default::default()
        });

        // fixme: this won't do on web
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: config.power_preference,
            ..Default::default()
        }))?;

        let adapter_info = adapter.get_info();

        let supported_features = adapter.features();
        let supported_limits = adapter.limits();

        let enabled_features = wgpu::Features::default();
        let enabled_limits = wgpu::Limits::defaults();

        Ok(Self {
            config,
            instance,
            adapter,
            adapter_info,
            supported_features,
            supported_limits,
            enabled_features,
            enabled_limits,
        })
    }

    #[track_caller]
    pub fn request_features(&mut self, features: wgpu::Features) -> &mut Self {
        let unsupported = features.difference(self.supported_features);
        if !unsupported.is_empty() {
            panic!("Requested features that iare not supported by the adapter: {unsupported:?}");
        }

        self.enabled_features.insert(features);

        self
    }

    pub fn build(self) -> Result<WgpuContext, Error> {
        // fixme: this won't do on web
        let (device, queue) = pollster::block_on(async {
            // these might need to be modified

            let (device, queue) = self
                .adapter
                .request_device(&wgpu::DeviceDescriptor {
                    required_features: self.enabled_features,
                    required_limits: self.enabled_limits,
                    memory_hints: match self.config.memory_hints {
                        MemoryHints::Performance => wgpu::MemoryHints::Performance,
                        MemoryHints::MemoryUsage => wgpu::MemoryHints::MemoryUsage,
                    },
                    ..Default::default()
                })
                .await?;

            Ok::<_, Error>((device, queue))
        })?;

        let info = WgpuInfo {
            adapter: self.adapter_info,
            features: device.features(),
            limits: device.limits(),
        };

        let staging_pool = StagingPool::new(self.config.staging_chunk_size, "staging pool");

        Ok(WgpuContext {
            instance: self.instance,
            adapter: self.adapter,
            device,
            queue,
            staging_pool,
            info: Arc::new(info),
        })
    }
}

#[derive(Clone, Debug, Resource)]
pub struct WgpuContext {
    pub instance: wgpu::Instance,
    pub adapter: wgpu::Adapter,
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub staging_pool: StagingPool,
    pub info: Arc<WgpuInfo>,
}

#[derive(Clone, Debug)]
pub struct WgpuInfo {
    pub adapter: wgpu::AdapterInfo,
    pub features: wgpu::Features,
    pub limits: wgpu::Limits,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub enum MemoryHints {
    #[default]
    Performance,
    MemoryUsage,
}

pub fn create_texture(
    label: &str,
    size: &Vector2<u32>,
    usage: wgpu::TextureUsages,
    format: wgpu::TextureFormat,
    mip_level_count: NonZero<u32>,
    device: &wgpu::Device,
) -> wgpu::Texture {
    device.create_texture(&texture_descriptor(
        label,
        size,
        usage,
        format,
        mip_level_count,
    ))
}

/// Creates a 1 by 1 pixel texture from the given color
pub fn create_texture_from_linsrgba<S>(
    color: LinSrgba<u8>,
    usage: wgpu::TextureUsages,
    label: &str,
    device: &wgpu::Device,
    mut write_staging: S,
) -> wgpu::Texture
where
    S: WriteStaging,
{
    let size = Vector2::repeat(1);

    let texture = create_texture(
        label,
        &size,
        usage | wgpu::TextureUsages::COPY_DST,
        wgpu::TextureFormat::Rgba8Unorm,
        const { NonZero::new(1).unwrap() },
        device,
    );

    let mut view = write_staging.write_texture(
        TextureSourceLayout {
            // this must be padded
            bytes_per_row: wgpu::COPY_BYTES_PER_ROW_ALIGNMENT,
            rows_per_image: None,
        },
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: Default::default(),
            aspect: Default::default(),
        },
        wgpu::Extent3d {
            width: 1,
            height: 1,
            depth_or_array_layers: 1,
        },
    );

    let color: [u8; 4] = color.into();
    view[..4].copy_from_slice(&color);

    texture
}

pub fn texture_descriptor<'a>(
    label: &'a str,
    size: &Vector2<u32>,
    usage: wgpu::TextureUsages,
    format: wgpu::TextureFormat,
    mip_level_count: NonZero<u32>,
) -> wgpu::TextureDescriptor<'a> {
    wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size.x,
            height: size.y,
            depth_or_array_layers: 1,
        },
        mip_level_count: mip_level_count.get(),
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage,
        view_formats: &[],
    }
}

/// Layout of a texture in a buffer's memory.
///
/// This is [`TexelCopyBufferLayout`](wgpu::TexelCopyBufferLayout), but without
/// offset
#[derive(Clone, Copy, Debug)]
pub struct TextureSourceLayout {
    pub bytes_per_row: u32,
    pub rows_per_image: Option<u32>,
}

impl TextureSourceLayout {
    pub fn into_texel_copy_buffer_info<'buffer>(
        self,
        buffer_slice: wgpu::BufferSlice<'buffer>,
    ) -> wgpu::TexelCopyBufferInfo<'buffer> {
        wgpu::TexelCopyBufferInfo {
            buffer: buffer_slice.buffer(),
            layout: wgpu::TexelCopyBufferLayout {
                offset: buffer_slice.offset(),
                bytes_per_row: Some(self.bytes_per_row),
                rows_per_image: self.rows_per_image,
            },
        }
    }
}
