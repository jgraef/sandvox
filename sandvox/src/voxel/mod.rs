pub mod chunk;
pub mod chunk_generator;
pub mod chunk_map;
pub mod loader;
pub mod mesh;

use std::fmt::Debug;

pub trait Voxel: Clone + Debug + Send + Sync + 'static {}

pub trait VoxelData<V>: Clone + Send + Sync + 'static {
    fn texture(&self, voxel: &V, face: BlockFace) -> Option<u32>;
    fn is_opaque(&self, voxel: &V) -> bool;
    fn can_merge(&self, first: &V, second: &V) -> bool;
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
