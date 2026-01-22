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
        let sprite_id = self.insert(sprite);
        self.ui.insert(name, sprite_id);
        sprite_id
    }

    fn insert_icon(&mut self, name: String, sprite: Sprite) -> SpriteId {
        let sprite_id = self.insert(sprite);
        self.icons.insert(name, sprite_id);
        sprite_id
    }

    fn insert_crosshair(&mut self, sprite: Sprite) -> SpriteId {
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
}

impl Sprite {
    pub fn from_atlas(atlas_handle: AtlasHandle) -> Self {
        Self {
            atlas_handle,
            margin: None,
            content_margin: None,
            expand_margin: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
pub struct Margin {
    pub left: f32,
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
}

#[derive(Clone, Debug, Component)]
pub struct Background {
    pub sprite: Sprite,
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
            let content_offset = Point2::new(
                rounded_layout.content_box_x(),
                rounded_layout.content_box_y(),
            );
            let content_size = Vector2::new(
                rounded_layout.content_box_width(),
                rounded_layout.content_box_height(),
            );

            tracing::trace!(?entity, sprite = ?background.sprite, ?content_offset, ?content_size, "render background");

            render_buffer_builder
                .push_quad(content_offset, content_size, rounded_layout.order, None)
                .set_atlas_texture(&background.sprite.atlas_handle);
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

        let atlas_id = atlas.insert_image(
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
            tracing::debug!(?atlas_id, ?name, ?entry.margin, ?entry.content_margin, ?entry.expand_margin, "nine-patch?");
        }

        insert_sprite(
            name,
            Sprite {
                atlas_handle: atlas_id,
                margin: entry.margin,
                content_margin: entry.content_margin,
                expand_margin: entry.expand_margin,
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
