// todo: should this be moved? maybe into crate::world?

use std::{
    collections::HashMap,
    ops::Index,
    path::Path,
};

use bevy_ecs::resource::Resource;
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
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlockType(usize);

#[derive(Clone, Debug, Resource)]
pub struct BlockTypes {
    blocks: Vec<BlockTypeData>,
    by_name: HashMap<String, usize>,
}

impl BlockTypes {
    pub fn load(path: impl AsRef<Path>, atlas_builder: &mut AtlasBuilder) -> Result<Self, Error> {
        let toml_directory = path.as_ref().parent().unwrap();
        let toml = std::fs::read(&path)?;
        let block_defs: config::BlockDefs = toml::from_slice(&toml)?;

        let mut blocks = Vec::with_capacity(block_defs.block_defs.len());
        let mut by_name = HashMap::with_capacity(block_defs.block_defs.len());

        for (i, (name, block_def)) in block_defs.block_defs.into_iter().enumerate() {
            let texture_id = block_def
                .texture
                .map(|path| {
                    let path = toml_directory.join(path);
                    let image =
                        RgbaImage::from_path(&path).with_note(|| path.display().to_string())?;
                    let texture_id = atlas_builder.insert(&image)?;
                    Ok::<_, Error>(texture_id)
                })
                .transpose()?;

            by_name.insert(name.clone(), i);
            blocks.push(BlockTypeData {
                name,
                texture_id,
                is_opaque: block_def.is_opaque,
            });
        }

        for (i, data) in blocks.iter().enumerate() {
            tracing::debug!("block_type: {i} => {}", data.name);
        }

        Ok(Self { blocks, by_name })
    }

    pub fn lookup(&self, name: &str) -> Option<BlockType> {
        Some(BlockType(*self.by_name.get(name)?))
    }
}

impl Index<BlockType> for BlockTypes {
    type Output = BlockTypeData;

    fn index(&self, index: BlockType) -> &Self::Output {
        &self.blocks[index.0]
    }
}

#[derive(Clone, Debug)]
pub struct BlockTypeData {
    pub name: String,
    pub texture_id: Option<AtlasId>,
    pub is_opaque: bool,
}

mod config {
    use std::path::PathBuf;

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
    pub struct BlockDef {
        pub texture: Option<PathBuf>,

        #[serde(default = "default_true")]
        pub is_opaque: bool,
    }
}
