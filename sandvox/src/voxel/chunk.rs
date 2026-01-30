use std::{
    ops::Index,
    sync::Arc,
};

use bevy_ecs::component::Component;
use morton_encoding::{
    morton_decode,
    morton_encode,
};
use nalgebra::Point3;

/// A 3D array of voxels.
///
/// The size of the chunk is determined at compile time and needs to be a power
/// of 2.
///
/// The chunk data itself is reference-counted. Thus cloning the [`Chunk`] is
/// cheap. Modification might copy the data if there are multiple references to
/// the chunk. (Todo: modification is not implemented yet)
///
/// Internally the data is layout in Z-order to improve cache coherency.
#[derive(derive_more::Debug, Clone, Component)]
pub struct Chunk<V, const CHUNK_SIZE: usize> {
    #[debug(skip)]
    voxels: Arc<[V]>,
}

impl<V, const CHUNK_SIZE: usize> Chunk<V, CHUNK_SIZE> {
    pub fn from_fn(mut f: impl FnMut(Point3<u16>) -> V) -> Self {
        let num_voxels = CHUNK_SIZE * CHUNK_SIZE * CHUNK_SIZE;

        // note: according to the [docs][1], if the iterator implements `TrustedLen`
        // (which our's does), this will only do one allocation.
        //
        // [1]: https://doc.rust-lang.org/std/sync/struct.Arc.html#impl-FromIterator%3CT%3E-for-Arc%3C%5BT%5D%3E
        let voxels = (0..num_voxels)
            .map(|i| {
                let point = Point3::from(morton_decode::<u16, 3>(i.try_into().unwrap()));
                f(point)
            })
            .collect::<Arc<[V]>>();

        Self { voxels }
    }

    pub fn iter(&self) -> impl Iterator<Item = (Point3<u16>, &V)> {
        self.voxels.iter().enumerate().map(|(i, voxel)| {
            let point = Point3::from(morton_decode::<u16, 3>(i.try_into().unwrap()));
            (point, voxel)
        })
    }

    #[inline]
    pub fn byte_size(&self) -> usize {
        size_of::<V>() * self.voxels.len()
    }
}

impl<V, const CHUNK_SIZE: usize> Index<Point3<u16>> for Chunk<V, CHUNK_SIZE> {
    type Output = V;

    #[inline]
    fn index(&self, index: Point3<u16>) -> &V {
        &self.voxels[morton_encode(index.into()) as usize]
    }
}

impl<V, const CHUNK_SIZE: usize> AsRef<[V]> for Chunk<V, CHUNK_SIZE> {
    #[inline]
    fn as_ref(&self) -> &[V] {
        &self.voxels
    }
}
