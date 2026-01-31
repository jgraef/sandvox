use std::{
    collections::{
        HashMap,
        hash_map,
    },
    ops::Index,
    path::Path,
};

use bevy_ecs::{
    component::Component,
    name::NameOrEntity,
    query::Changed,
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
        DefaultAtlas,
        RenderSystems,
        atlas::{
            Atlas,
            AtlasHandle,
            Padding,
            PaddingFill,
            PaddingMode,
        },
        staging::Staging,
    },
    ui::{
        FinalLayout,
        RenderBufferBuilder,
        Root,
        UiSystems,
        sprites::ui_defs::MarginDef,
        view::View,
    },
    util::image::ImageLoadExt,
    wgpu::WgpuContext,
};

#[derive(Debug, Default, Resource)]
pub struct Sprites {
    sprites: Vec<Sprite>,
    by_name: HashMap<String, SpriteId>,
}

impl Sprites {
    fn insert(&mut self, name: String, sprite: Sprite) -> SpriteId {
        let sprite_id = SpriteId(self.sprites.len());
        self.sprites.push(sprite);
        self.by_name.insert(name, sprite_id);
        sprite_id
    }

    pub fn lookup(&self, name: &str) -> Option<SpriteId> {
        self.by_name.get(name).copied()
    }

    pub fn load(
        path: impl AsRef<Path>,
        device: &wgpu::Device,
        atlas: &mut Atlas,
        staging: &mut Staging,
    ) -> Result<Self, Error> {
        let toml_directory = path.as_ref().parent().unwrap();
        let toml = std::fs::read(&path)?;
        let ui_defs: ui_defs::SpriteDefs = toml::from_slice(&toml)?;

        let mut image_cache = HashMap::new();
        let mut sprites = Sprites::default();

        for (name, sprite_def) in ui_defs.sprites {
            let image = match image_cache.entry(sprite_def.source.clone()) {
                hash_map::Entry::Occupied(occupied) => occupied.into_mut(),
                hash_map::Entry::Vacant(vacant) => {
                    let image = RgbaImage::from_path(toml_directory.join(&sprite_def.source))?;
                    vacant.insert(image)
                }
            };

            let sub_image = image
                .view(
                    sprite_def.x,
                    sprite_def.y,
                    sprite_def.width,
                    sprite_def.height,
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

            let mut nine_patch = None;
            let mut padding = None;

            if let Some(margin) = sprite_def.nine_patch {
                let margin = match margin {
                    MarginDef::SingleMargin { margin } => {
                        Margin {
                            left: margin,
                            top: margin,
                            right: margin,
                            bottom: margin,
                        }
                    }
                };

                nine_patch = Some(NinePatch::new(&atlas_handle, atlas, margin));
                padding = Some(margin);
            };

            sprites.insert(
                name,
                Sprite {
                    atlas_handle,
                    nine_patch,
                    padding,
                    size: Vector2::new(sprite_def.width, sprite_def.height),
                },
            );
        }

        Ok(sprites)
    }
}

impl Index<SpriteId> for Sprites {
    type Output = Sprite;

    fn index(&self, index: SpriteId) -> &Self::Output {
        &self.sprites[index.0]
    }
}

impl Index<&str> for Sprites {
    type Output = Sprite;

    fn index(&self, index: &str) -> &Self::Output {
        let sprite_id = *self
            .by_name
            .get(index)
            .unwrap_or_else(|| panic!("No such sprite: {index}"));
        &self[sprite_id]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SpriteId(usize);

#[derive(Clone, Debug)]
pub struct Sprite {
    pub atlas_handle: AtlasHandle,
    pub nine_patch: Option<NinePatch>,
    pub padding: Option<Margin>,
    pub size: Vector2<u32>,
}

impl Sprite {
    pub fn padding(&self, pixel_size: f32) -> Option<taffy::Rect<taffy::LengthPercentage>> {
        self.padding.map(|padding| {
            taffy::Rect {
                left: taffy::LengthPercentage::length(padding.left as f32 * pixel_size),
                right: taffy::LengthPercentage::length(padding.right as f32 * pixel_size),
                top: taffy::LengthPercentage::length(padding.top as f32 * pixel_size),
                bottom: taffy::LengthPercentage::length(padding.bottom as f32 * pixel_size),
            }
        })
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
        depth: u32,
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
                        depth,
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
    let path = Path::new("assets/ui.toml");
    let sprites = Sprites::load(path, &wgpu.device, &mut atlas.0, &mut *staging).unwrap();
    commands.insert_resource(sprites);
}

fn request_redraw(nodes: Populated<&Root, Changed<Background>>, mut views: Populated<&mut View>) {
    for root in nodes {
        let mut view = views.get_mut(root.root).unwrap();
        view.render = true;
    }
}

fn render_sprites(
    nodes: Populated<(NameOrEntity, &Background, &FinalLayout, &Root)>,
    mut views: Populated<(&View, &mut RenderBufferBuilder)>,
) {
    for (entity, background, final_layout, root) in nodes {
        let (view, mut render_buffer_builder) = views.get_mut(root.root).unwrap();

        if view.render {
            let offset = Point2::new(final_layout.location.x, final_layout.location.y);
            let size = Vector2::new(final_layout.size.width, final_layout.size.height);

            tracing::trace!(
                %entity,
                ?background,
                ?offset,
                ?size,
                depth = ?final_layout.depth,
                "render background"
            );

            if let Some(nine_patch) = &background.sprite.nine_patch {
                nine_patch.render(
                    &mut render_buffer_builder,
                    offset,
                    size,
                    final_layout.depth,
                    background.pixel_size,
                );
            }
            else {
                render_buffer_builder
                    .push_quad(offset, size, final_layout.depth, None)
                    .set_atlas_texture(&background.sprite.atlas_handle);
            }
        }
    }
}

mod ui_defs {
    use std::path::PathBuf;

    use indexmap::IndexMap;
    use serde::Deserialize;

    #[derive(Debug, Deserialize)]
    #[serde(transparent)]
    pub struct SpriteDefs {
        pub sprites: IndexMap<String, SpriteDef>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct SpriteDef {
        pub source: PathBuf,
        pub x: u32,
        pub y: u32,
        pub width: u32,
        pub height: u32,
        pub nine_patch: Option<MarginDef>,
    }

    #[derive(Debug, Deserialize)]
    #[serde(untagged, deny_unknown_fields)]
    pub enum MarginDef {
        SingleMargin { margin: u32 },
    }
}
