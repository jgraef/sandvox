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
    type FetchData: SystemParam;
    type Data: for<'a, 'w, 's> From<&'a <Self::FetchData as SystemParam>::Item<'w, 's>>
        + Clone
        + Send
        + Sync
        + 'static;

    fn texture<'w, 's>(&self, face: BlockFace, data: &Self::Data) -> Option<AtlasId>;

    fn is_opaque<'w, 's>(&self, data: &Self::Data) -> bool;

    fn can_merge<'w, 's>(&self, other: &Self, data: &Self::Data) -> bool;
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
