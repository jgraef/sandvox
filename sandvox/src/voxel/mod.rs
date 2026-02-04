pub mod chunk;
pub mod chunk_generator;
pub mod chunk_map;
pub mod loader;
pub mod mesh;

use std::fmt::Debug;

use nalgebra::Vector3;

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

    #[inline]
    pub fn neighbor(&self) -> Vector3<i16> {
        match self {
            BlockFace::Left => Vector3::new(-1, 0, 0),
            BlockFace::Right => Vector3::new(1, 0, 0),
            BlockFace::Down => Vector3::new(0, -1, 0),
            BlockFace::Up => Vector3::new(0, 1, 0),
            BlockFace::Front => Vector3::new(0, 0, -1),
            BlockFace::Back => Vector3::new(0, 0, 1),
        }
    }
}
