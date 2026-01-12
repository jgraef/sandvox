pub mod chunk;
pub mod chunk_generator;
pub mod chunk_map;
pub mod loader;
pub mod mesh;
pub mod svo;

use std::fmt::Debug;

use bevy_ecs::system::SystemParam;

use crate::render::texture_atlas::AtlasId;

pub trait Voxel: Clone + Debug + Send + Sync + 'static {
    type Data: SystemParam;

    fn texture<'w, 's>(
        &self,
        face: BlockFace,
        data: &<Self::Data as SystemParam>::Item<'w, 's>,
    ) -> Option<AtlasId>;

    fn is_opaque<'w, 's>(&self, data: &<Self::Data as SystemParam>::Item<'w, 's>) -> bool;

    fn can_merge<'w, 's>(
        &self,
        other: &Self,
        data: &<Self::Data as SystemParam>::Item<'w, 's>,
    ) -> bool;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum BlockFace {
    Left = 0,
    Right = 1,
    Down = 2,
    Up = 3,
    Front = 4,
    Back = 5,
}

impl BlockFace {
    pub const ALL: [Self; 6] = [
        Self::Left,
        Self::Right,
        Self::Down,
        Self::Up,
        Self::Front,
        Self::Back,
    ];
}
