// todo: should this be moved? maybe into crate::world?

use std::{
    collections::HashMap,
    ops::Index,
    path::Path,
    sync::Arc,
};

use arrayvec::ArrayVec;
use bevy_ecs::{
    resource::Resource,
    system::Res,
};
use color_eyre::{
    Section,
    eyre::Error,
};
use image::RgbaImage;

use crate::{
    render::texture_atlas::{
        AtlasBuilder,
        AtlasId,
    },
    util::image::ImageLoadExt,
    voxel::BlockFace,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockType(u32);

impl BlockType {
    fn from_usize(i: usize) -> Self {
        let id = u32::try_from(i).expect("block type overflow");
        Self(id)
    }
}

#[derive(Clone, Debug, Resource)]
pub struct BlockTypes {
    inner: Arc<Inner>,
}

#[derive(Clone, Debug)]
struct Inner {
    blocks: Vec<BlockTypeData>,
    by_name: HashMap<String, BlockType>,
}

impl BlockTypes {
    pub fn load(path: impl AsRef<Path>, atlas_builder: &mut AtlasBuilder) -> Result<Self, Error> {
        let toml_directory = path.as_ref().parent().unwrap();
        let toml = std::fs::read(&path)?;
        let block_defs: config::BlockDefs = toml::from_slice(&toml)?;

        let mut blocks = Vec::with_capacity(block_defs.block_defs.len());
        let mut by_name = HashMap::with_capacity(block_defs.block_defs.len());

        let mut texture_cache = HashMap::new();

        for (i, (name, mut block_def)) in block_defs.block_defs.into_iter().enumerate() {
            if block_def.texture.is_none() && block_def.is_opaque {
                tracing::warn!("Block without texture defined as opaque: {name}");
                block_def.is_opaque = false;
            }

            let mut textures = None;

            if let Some(texture_def) = block_def.texture {
                let mut faces = ArrayVec::new();

                for path in texture_def.faces() {
                    let texture_id = if let Some(texture_id) = texture_cache.get(path) {
                        *texture_id
                    }
                    else {
                        let full_path = toml_directory.join(path);
                        let image = RgbaImage::from_path(&full_path)
                            .with_note(|| path.display().to_string())?;

                        let texture_id = atlas_builder.insert(&image)?;

                        tracing::debug!(?full_path, ?texture_id, "loaded texture");

                        texture_cache.insert(path.to_owned(), texture_id);
                        texture_id
                    };

                    faces.push(texture_id)
                }

                textures = Some(faces.into_inner().unwrap());
            }

            by_name.insert(name.clone(), BlockType::from_usize(i));
            blocks.push(BlockTypeData {
                name,
                textures,
                is_opaque: block_def.is_opaque,
            });
        }

        for (i, data) in blocks.iter().enumerate() {
            tracing::debug!("block_type: {i} => {}", data.name);
        }

        Ok(Self {
            inner: Arc::new(Inner { blocks, by_name }),
        })
    }

    pub fn lookup(&self, name: &str) -> Option<BlockType> {
        Some(*self.inner.by_name.get(name)?)
    }
}

impl Index<BlockType> for BlockTypes {
    type Output = BlockTypeData;

    fn index(&self, index: BlockType) -> &Self::Output {
        &self.inner.blocks[index.0 as usize]
    }
}

impl<'a, 'w> From<&'a Res<'w, BlockTypes>> for BlockTypes {
    fn from(value: &'a Res<'w, BlockTypes>) -> Self {
        (*value).clone()
    }
}

#[derive(Clone, Debug)]
pub struct BlockTypeData {
    pub name: String,
    pub textures: Option<[AtlasId; 6]>,
    pub is_opaque: bool,
}

impl BlockTypeData {
    pub fn face_texture(&self, face: BlockFace) -> Option<AtlasId> {
        self.textures
            .as_ref()
            .map(|faces| faces[usize::from(face as u8)])
    }
}

mod config {
    use std::path::{
        Path,
        PathBuf,
    };

    use indexmap::IndexMap;
    use serde::{
        Deserialize,
        Serialize,
    };

    use crate::util::serde::default_true;

    #[derive(Clone, Debug, Serialize, Deserialize)]
    #[serde(transparent)]
    pub struct BlockDefs {
        pub block_defs: IndexMap<String, BlockDef>,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    #[serde(deny_unknown_fields)]
    pub struct BlockDef {
        pub texture: Option<TextureDef>,

        #[serde(default = "default_true")]
        pub is_opaque: bool,
    }

    #[derive(Clone, Debug, Serialize, Deserialize)]
    #[serde(untagged)]
    pub enum TextureDef {
        Single(PathBuf),
        Faces {
            default: Option<PathBuf>,
            left: Option<PathBuf>,
            right: Option<PathBuf>,
            #[serde(alias = "bottom")]
            down: Option<PathBuf>,
            #[serde(alias = "top")]
            up: Option<PathBuf>,
            front: Option<PathBuf>,
            back: Option<PathBuf>,
        },
    }

    impl TextureDef {
        pub fn faces(&self) -> [&Path; 6] {
            match self {
                TextureDef::Single(path_buf) => std::array::repeat(&path_buf),
                TextureDef::Faces {
                    default,
                    left,
                    right,
                    down,
                    up,
                    front,
                    back,
                } => {
                    macro_rules! faces {
                        ($($face:ident),*) => {
                            [$($face.as_deref().unwrap_or_else(|| {
                                default.as_deref().unwrap_or_else(|| {
                                    panic!(
                                        "Missing face '{}' and no default specified",
                                        stringify!($face)
                                    )
                                })
                            })),*]
                        };
                    }
                    faces!(left, right, down, up, front, back)
                }
            }
        }
    }
}
