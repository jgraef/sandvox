use std::ops::Index;

use bevy_ecs::component::Component;
use morton_encoding::{
    morton_decode,
    morton_encode,
};
use nalgebra::Point3;

#[derive(derive_more::Debug, Clone, Component)]
pub struct Chunk<V, const CHUNK_SIZE: usize> {
    #[debug(skip)]
    pub voxels: Box<[V]>,
}

impl<V, const CHUNK_SIZE: usize> Chunk<V, CHUNK_SIZE> {
    pub fn from_fn(mut f: impl FnMut(Point3<u16>) -> V) -> Self {
        let num_voxels = CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE;
        let mut voxels = Box::new_uninit_slice(num_voxels);

        // fixme: memory leak when f panics
        for (i, voxel) in voxels.iter_mut().enumerate() {
            let point = Point3::from(morton_decode::<u16, 3>(i.try_into().unwrap()));
            voxel.write(f(point));
        }

        let voxels = unsafe { voxels.assume_init() };

        Self { voxels }
    }
}

impl<V, const CHUNK_SIZE: usize> Index<Point3<u16>> for Chunk<V, CHUNK_SIZE> {
    type Output = V;

    fn index(&self, index: Point3<u16>) -> &V {
        &self.voxels[morton_encode(index.into()) as usize]
    }
}
