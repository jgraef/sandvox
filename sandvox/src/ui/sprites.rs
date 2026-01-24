use std::{
    collections::HashMap,
    fs::File,
    io::BufReader,
    ops::Index,
    path::Path,
};

use bevy_ecs::{
    component::Component,
    entity::Entity,
    query::{
        Changed,
        With,
    },
    resource::Resource,
    schedule::IntoScheduleConfigs,
    system::{
        Commands,
        Populated,
        Res,
        ResMut,
    },
};
use color_eyre::eyre::Error;
use image::{
    GenericImageView,
    RgbaImage,
};
use nalgebra::{
    Point2,
    Vector2,
};
use serde::Deserialize;

use crate::{
    ecs::{
        plugin::WorldBuilder,
        schedule,
    },
    render::{
        RenderSystems,
        atlas::{
            Atlas,
            AtlasHandle,
            Padding,
            PaddingFill,
            PaddingMode,
        },
        frame::DefaultAtlas,
        staging::Staging,
    },
    ui::{
        RedrawRequested,
        RenderBufferBuilder,
        Root,
        RoundedLayout,
        UiSystems,
    },
    util::image::ImageLoadExt,
    wgpu::WgpuContext,
};

#[derive(Debug, Default, Resource)]
pub struct Sprites {
    sprites: Vec<Sprite>,
    ui: HashMap<String, SpriteId>,
    icons: HashMap<String, SpriteId>,
    crosshairs: Vec<SpriteId>,
}

impl Sprites {
    fn insert(&mut self, sprite: Sprite) -> SpriteId {
        let sprite_id = SpriteId(self.sprites.len());
        self.sprites.push(sprite);
        sprite_id
    }

    fn insert_ui(&mut self, name: String, sprite: Sprite) -> SpriteId {
        tracing::debug!(?name, ?sprite, "loaded ui sprite");

        let sprite_id = self.insert(sprite);
        self.ui.insert(name, sprite_id);
        sprite_id
    }

    fn insert_icon(&mut self, name: String, sprite: Sprite) -> SpriteId {
        tracing::debug!(?name, ?sprite, "loaded icon sprite");

        let sprite_id = self.insert(sprite);
        self.icons.insert(name, sprite_id);
        sprite_id
    }

    fn insert_crosshair(&mut self, sprite: Sprite) -> SpriteId {
        tracing::debug!(?sprite, "loaded crosshair sprite");

        let sprite_id = self.insert(sprite);
        self.crosshairs.push(sprite_id);
        sprite_id
    }

    pub fn lookup_ui(&self, name: &str) -> Option<SpriteId> {
        self.ui.get(name).copied()
    }

    pub fn lookup_icon(&self, name: &str) -> Option<SpriteId> {
        self.icons.get(name).copied()
    }

    pub fn lookup_crosshair(&self, index: usize) -> Option<SpriteId> {
        self.crosshairs.get(index).copied()
    }
}

impl Index<SpriteId> for Sprites {
    type Output = Sprite;

    fn index(&self, index: SpriteId) -> &Self::Output {
        &self.sprites[index.0]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SpriteId(usize);

#[derive(Clone, Debug)]
pub struct Sprite {
    pub atlas_handle: AtlasHandle,
    pub margin: Option<Margin>,
    pub content_margin: Option<Margin>,
    pub expand_margin: Option<Margin>,
    pub nine_patch: Option<NinePatch>,
}

impl Sprite {
    pub fn from_atlas(atlas_handle: AtlasHandle) -> Self {
        Self {
            atlas_handle,
            margin: None,
            content_margin: None,
            expand_margin: None,
            nine_patch: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct Margin {
    pub left: u32,
    pub top: u32,
    pub right: u32,
    pub bottom: u32,
}

impl Margin {
    pub fn to_padding(&self, pixel_size: f32) -> taffy::Rect<taffy::LengthPercentage> {
        tracing::debug!(
            left = self.left as f32 * pixel_size,
            right = self.right as f32 * pixel_size,
            top = self.top as f32 * pixel_size,
            bottom = self.bottom as f32 * pixel_size,
            "padding"
        );

        taffy::Rect {
            left: taffy::LengthPercentage::length(self.left as f32 * pixel_size),
            right: taffy::LengthPercentage::length(self.right as f32 * pixel_size),
            top: taffy::LengthPercentage::length(self.top as f32 * pixel_size),
            bottom: taffy::LengthPercentage::length(self.bottom as f32 * pixel_size),
        }
    }
}

#[derive(Clone, Debug, Component)]
pub struct Background {
    pub sprite: Sprite,
    pub pixel_size: f32,
}

#[derive(Clone, Debug)]
pub struct NinePatch {
    patches: [[AtlasHandle; 3]; 3],
    margin: Margin,
}

impl NinePatch {
    pub fn new(atlas_handle: &AtlasHandle, atlas: &mut Atlas, margin: Margin) -> Self {
        let size = atlas.view_size(atlas_handle);

        let mut patch = |mut left, mut top, mut right, mut bottom| {
            if left > right {
                std::mem::swap(&mut left, &mut right);
            }
            if top > bottom {
                std::mem::swap(&mut bottom, &mut top)
            }

            let patch_size = Vector2::new(right - left, bottom - top);
            let texture = atlas.view(atlas_handle, Vector2::new(left, top), patch_size);
            texture
        };

        let patches = [
            [
                patch(0, 0, margin.left, margin.top),
                patch(margin.left, 0, size.x - margin.right, margin.top),
                patch(size.x - margin.right, 0, size.x, margin.top),
            ],
            [
                patch(0, margin.top, margin.left, size.y - margin.bottom),
                patch(
                    margin.left,
                    margin.top,
                    size.x - margin.right,
                    size.y - margin.bottom,
                ),
                patch(
                    size.x - margin.right,
                    margin.top,
                    size.x,
                    size.y - margin.bottom,
                ),
            ],
            [
                patch(0, size.y - margin.bottom, margin.left, size.y),
                patch(
                    margin.left,
                    size.y - margin.bottom,
                    size.x - margin.right,
                    size.y,
                ),
                patch(
                    size.x - margin.right,
                    size.y - margin.bottom,
                    size.x,
                    size.y,
                ),
            ],
        ];

        NinePatch { patches, margin }
    }

    pub fn render(
        &self,
        render_buffer_builder: &mut RenderBufferBuilder,
        offset: Point2<f32>,
        size: Vector2<f32>,
        order: u32,
        pixel_size: f32,
    ) {
        fn patch_sizes(size: f32, margin_low: f32, margin_high: f32) -> [f32; 3] {
            let mut spacings = [0.0; 3];
            spacings[0] = margin_low.min(size);
            spacings[2] = margin_high.clamp(0.0, size - spacings[0]);
            spacings[1] = (size - spacings[0] - spacings[2]).max(0.0);
            spacings
        }

        let mut horizontal = patch_sizes(
            size.x / pixel_size,
            self.margin.left as f32,
            self.margin.right as f32,
        );
        let mut vertical = patch_sizes(
            size.y / pixel_size,
            self.margin.top as f32,
            self.margin.bottom as f32,
        );
        for i in 0..3 {
            horizontal[i] *= pixel_size;
            vertical[i] *= pixel_size;
        }

        let mut cursor = offset;

        for y in 0..3 {
            for x in 0..3 {
                render_buffer_builder
                    .push_quad(
                        cursor,
                        Vector2::new(horizontal[x], vertical[y]),
                        order,
                        None,
                    )
                    .set_atlas_texture(&self.patches[y][x]);
                cursor.x += horizontal[x];
            }
            cursor.x = offset.x;
            cursor.y += vertical[y];
        }
    }
}

pub(super) fn setup_sprite_systems(builder: &mut WorldBuilder) {
    builder
        .add_systems(schedule::Startup, load_sprites.in_set(RenderSystems::Setup))
        .add_systems(
            schedule::Render,
            (
                request_redraw.before(UiSystems::Render),
                render_sprites.in_set(UiSystems::Render),
            ),
        );
}

fn load_sprites(
    wgpu: Res<WgpuContext>,
    mut atlas: ResMut<DefaultAtlas>,
    mut staging: ResMut<Staging>,
    mut commands: Commands,
) {
    // todo: hard-coded asset path
    let sprites = load_all_sprites("assets", &mut atlas, &wgpu.device, &mut *staging).unwrap();

    commands.insert_resource(sprites);
}

fn request_redraw(nodes: Populated<&Root, Changed<Background>>, mut commands: Commands) {
    for root in nodes {
        commands.entity(root.viewport).insert(RedrawRequested);
    }
}

fn render_sprites(
    nodes: Populated<(Entity, &Background, &RoundedLayout, &Root)>,
    requested_redraw: Populated<(), With<RedrawRequested>>,
    mut surfaces: Populated<&mut RenderBufferBuilder>,
) {
    for (entity, background, rounded_layout, root) in nodes {
        // - check if the root of the ui tree is requested to be redrawn
        // - get the render target
        // - get the render buffer builder for the render target
        if let Ok(()) = requested_redraw.get(root.viewport)
            && let Some(render_target) = root.render_target
            && let Ok(mut render_buffer_builder) = surfaces.get_mut(render_target)
        {
            let content_offset = Point2::new(rounded_layout.location.x, rounded_layout.location.y);
            let content_size = Vector2::new(rounded_layout.size.width, rounded_layout.size.height);

            tracing::trace!(
                ?entity,
                ?background,
                ?content_offset,
                ?content_size,
                "render background"
            );

            if let Some(nine_patch) = &background.sprite.nine_patch {
                nine_patch.render(
                    &mut render_buffer_builder,
                    content_offset,
                    content_size,
                    rounded_layout.order,
                    background.pixel_size,
                );
            }
            else {
                render_buffer_builder
                    .push_quad(content_offset, content_size, rounded_layout.order, None)
                    .set_atlas_texture(&background.sprite.atlas_handle);
            }
        }
    }
}

fn load_all_sprites(
    path: impl AsRef<Path>,
    mut atlas: &mut Atlas,
    device: &wgpu::Device,
    mut staging: &mut Staging,
) -> Result<Sprites, Error> {
    let path = path.as_ref();
    let mut sprites = Sprites::default();

    load_sprite_sheet_json(
        path.join("ui.json"),
        path.join("ui.png"),
        &mut atlas,
        |name, sprite| {
            sprites.insert_ui(name, sprite);
        },
        device,
        &mut staging,
    )?;

    load_sprite_sheet_json(
        path.join("icons.json"),
        path.join("icons.png"),
        &mut atlas,
        |name, sprite| {
            sprites.insert_icon(name, sprite);
        },
        device,
        &mut staging,
    )?;

    load_sprite_sheet_tiled(
        Vector2::repeat(7),
        path.join("crosshairs.png"),
        &mut atlas,
        |sprite| {
            sprites.insert_crosshair(sprite);
        },
        device,
        &mut staging,
    )?;

    Ok(sprites)
}

fn load_sprite_sheet_json(
    json_path: impl AsRef<Path>,
    image_path: impl AsRef<Path>,
    atlas: &mut Atlas,
    mut insert_sprite: impl FnMut(String, Sprite),
    device: &wgpu::Device,
    staging: &mut Staging,
) -> Result<(), Error> {
    #[derive(Debug, Deserialize)]
    struct Source {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    }

    #[derive(Debug, Deserialize)]
    struct Entry {
        source: Source,
        margin: Option<Margin>,
        content_margin: Option<Margin>,
        expand_margin: Option<Margin>,
    }

    let reader = BufReader::new(File::open(json_path)?);
    let entries: HashMap<String, Entry> = serde_json::from_reader(reader)?;

    let image = RgbaImage::from_path(image_path)?;

    for (name, entry) in entries {
        let sub_image = image
            .view(
                entry.source.x,
                entry.source.y,
                entry.source.width,
                entry.source.height,
            )
            .to_image();

        let atlas_handle = atlas.insert_image(
            &sub_image,
            Some(PaddingMode {
                padding: Padding::uniform(1),
                fill: PaddingFill::TRANSPARENT,
            }),
            device,
            staging,
        )?;

        if entry.margin.is_some() || entry.content_margin.is_some() || entry.expand_margin.is_some()
        {
            tracing::debug!(?atlas_handle, ?name, ?entry.margin, ?entry.content_margin, ?entry.expand_margin, "nine-patch?");
        }

        let nine_patch = entry
            .margin
            .map(|margin| NinePatch::new(&atlas_handle, atlas, margin));

        insert_sprite(
            name,
            Sprite {
                atlas_handle,
                margin: entry.margin,
                content_margin: entry.content_margin,
                expand_margin: entry.expand_margin,
                nine_patch,
            },
        );
    }

    Ok(())
}

fn load_sprite_sheet_tiled(
    sprite_size: Vector2<u32>,
    image_path: impl AsRef<Path>,
    atlas: &mut Atlas,
    mut insert_sprite: impl FnMut(Sprite),
    device: &wgpu::Device,
    staging: &mut Staging,
) -> Result<(), Error> {
    let image = RgbaImage::from_path(image_path)?;

    for y in 0..(image.height() / sprite_size.y) {
        for x in 0..(image.width() / sprite_size.x) {
            let sub_image = image
                .view(
                    x * sprite_size.x,
                    y * sprite_size.y,
                    sprite_size.x,
                    sprite_size.y,
                )
                .to_image();

            let atlas_id = atlas.insert_image(
                &sub_image,
                Some(PaddingMode {
                    padding: Padding::uniform(1),
                    fill: PaddingFill::TRANSPARENT,
                }),
                device,
                staging,
            )?;

            insert_sprite(Sprite::from_atlas(atlas_id));
        }
    }

    Ok(())
}
