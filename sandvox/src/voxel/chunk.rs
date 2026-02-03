use std::{
    ops::Index,
    sync::Arc,
};

use bevy_ecs::component::Component;
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
pub struct Chunk<V, S> {
    #[debug(skip)]
    voxels: Arc<[V]>,
    shape: S,
}

impl<V, S> Chunk<V, S>
where
    S: ChunkShape,
{
    pub fn from_fn(shape: S, mut f: impl FnMut(Point3<u16>) -> V) -> Self {
        let side_length = shape.side_length();
        let num_voxels = side_length * side_length * side_length;

        // note: according to the [docs][1], if the iterator implements `TrustedLen`
        // (which our's does), this will only do one allocation.
        //
        // [1]: https://doc.rust-lang.org/std/sync/struct.Arc.html#impl-FromIterator%3CT%3E-for-Arc%3C%5BT%5D%3E
        let voxels = (0..num_voxels)
            .map(|i| f(shape.decode(i)))
            .collect::<Arc<[V]>>();

        Self { voxels, shape }
    }

    pub fn iter(&self) -> impl Iterator<Item = (Point3<u16>, &V)> {
        self.voxels
            .iter()
            .enumerate()
            .map(|(i, voxel)| (self.shape.decode(i), voxel))
    }
}

impl<V, S> Chunk<V, S> {
    #[inline]
    pub fn byte_size(&self) -> usize {
        size_of::<V>() * self.voxels.len()
    }

    #[inline]
    pub fn shape(&self) -> &S {
        &self.shape
    }
}

impl<V, S> Index<Point3<u16>> for Chunk<V, S>
where
    S: ChunkShape,
{
    type Output = V;

    #[inline]
    fn index(&self, index: Point3<u16>) -> &V {
        &self.voxels[self.shape.encode(index)]
    }
}

impl<V, S> AsRef<[V]> for Chunk<V, S> {
    #[inline]
    fn as_ref(&self) -> &[V] {
        &self.voxels
    }
}

pub trait ChunkShape: Clone + Send + Sync + 'static {
    fn side_length(&self) -> usize;
    fn encode(&self, point: Point3<u16>) -> usize;
    fn decode(&self, index: usize) -> Point3<u16>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct MortonShape<const CHUNK_SIZE: usize>;

impl<const CHUNK_SIZE: usize> ChunkShape for MortonShape<CHUNK_SIZE> {
    #[inline]
    fn side_length(&self) -> usize {
        assert!(CHUNK_SIZE.is_power_of_two());

        CHUNK_SIZE
    }

    #[inline]
    fn encode(&self, point: Point3<u16>) -> usize {
        morton::encode::<[u16; 3]>(point.into()) as usize
    }

    #[inline]
    fn decode(&self, index: usize) -> Point3<u16> {
        morton::decode::<[u16; 3]>(index as u64).into()
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct LinearShape<const CHUNK_SIZE: usize>;

impl<const CHUNK_SIZE: usize> ChunkShape for LinearShape<CHUNK_SIZE> {
    #[inline]
    fn side_length(&self) -> usize {
        CHUNK_SIZE
    }

    #[inline]
    fn encode(&self, point: Point3<u16>) -> usize {
        point.z as usize * CHUNK_SIZE * CHUNK_SIZE
            + point.y as usize * CHUNK_SIZE
            + point.x as usize
    }

    #[inline]
    fn decode(&self, index: usize) -> Point3<u16> {
        let z = (index / (CHUNK_SIZE * CHUNK_SIZE)) as u16;
        let r = index % (CHUNK_SIZE * CHUNK_SIZE);
        let y = (r / CHUNK_SIZE) as u16;
        let x = (r % CHUNK_SIZE) as u16;
        Point3::new(x, y, z)
    }
}
