use nalgebra::{
    Point2,
    Point3,
};

use crate::{
    util::bitmatrix_transpose::BitMatrix,
    voxel::{
        Voxel,
        VoxelData,
        chunk::{
            Chunk,
            ChunkShape,
        },
        mesh::opacity_mask::transpose::{
            ColumnView,
            OpacityMaskView,
            RowView,
        },
    },
};

/// Opacity masks for 3 axis: XY, ZY, XZ
#[derive(Debug)]
pub struct OpacityMasks {
    /// The outer array has one element per direction. They contain the same
    /// data (if a voxel is opaque or not), but a different axis is stored in
    /// the bits.
    ///
    /// The outer array corresponds to faces: XY, ZY, XZ. Or equivalently the
    /// bit masks correspond to stacks along Z, X and Y axis.
    ///
    /// Each inner array is in morton-order. Two axis (e.g. XY) are mapped to
    /// array elements. The third axis (e.g. Z) is represented by individual
    /// bits in the array entries.
    xy: Box<[u64]>,
    zy: Box<[u64]>,
    xz: Box<[u64]>,
}

impl OpacityMasks {
    pub fn new<S>(shape: &S) -> Self
    where
        S: ChunkShape,
    {
        // This is rather large (288 KiB for 64^3 chunks) so it is
        // heap-allocated.
        //
        // The implementation of [`Default` for `Box`][1] seems to not
        // construct the value on the stack and then move it, but
        // initialize it on the heap directly - which is desired.
        //
        // Unfortunately for some reason `Default` is not implemented for large
        // arrays, so we can't use that. Let's just hope this is optimized.
        //
        // [1]: https://doc.rust-lang.org/src/alloc/boxed.rs.html#1694

        let side_length = shape.side_length();
        let num_voxels = side_length * side_length * side_length;

        Self {
            xy: vec![0; num_voxels].into_boxed_slice(),
            zy: vec![0; num_voxels].into_boxed_slice(),
            xz: vec![0; num_voxels].into_boxed_slice(),
        }
    }

    #[profiling::function]
    pub fn fill<V, S, D>(&mut self, chunk: &Chunk<V, S>, data: &D)
    where
        V: Voxel,
        S: ChunkShape,
        D: VoxelData<V>,
    {
        let chunk_size = chunk.shape().side_length();

        // fill XY opacity matrix
        for i in 0..(chunk_size * chunk_size) {
            let [x, y] = morton::decode::<[u16; 2]>(i.try_into().unwrap());
            let mut mask_i = 0;
            for z in 0..chunk_size as u16 {
                if data.is_opaque(&chunk[Point3::new(x, y, z)]) {
                    mask_i |= 1 << z;
                }
            }
            self.xy[i] = mask_i;
        }

        // flip X and Z
        self.zy.copy_from_slice(&self.xy);
        for y in 0..chunk_size as u16 {
            OpacityMaskView {
                mask: &mut self.zy,
                side_length: chunk_size,
                view: RowView { y },
            }
            .transpose();
        }

        // flip Y and Z
        self.xz.copy_from_slice(&self.xy);
        for x in 0..chunk_size as u16 {
            OpacityMaskView {
                mask: &mut self.xz,
                side_length: chunk_size,
                view: ColumnView { x },
            }
            .transpose();
        }
    }

    #[inline]
    pub fn opacity_xy(&self, xy: Point2<u16>) -> OpacityMask {
        let i: usize = morton::encode::<[u16; 2]>(xy.into()).try_into().unwrap();
        OpacityMask(self.xy[i])
    }

    #[inline]
    pub fn opacity_zy(&self, zy: Point2<u16>) -> OpacityMask {
        let i: usize = morton::encode::<[u16; 2]>(zy.into()).try_into().unwrap();
        OpacityMask(self.zy[i])
    }

    #[inline]
    pub fn opacity_xz(&self, xz: Point2<u16>) -> OpacityMask {
        let i: usize = morton::encode::<[u16; 2]>(xz.into()).try_into().unwrap();
        OpacityMask(self.xz[i])
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct OpacityMask(pub u64);

impl OpacityMask {
    #[inline]
    pub fn get(&self, i: u16) -> bool {
        self.0 & (1 << i) != 0
    }

    #[inline]
    pub fn front_face_mask(&self) -> u64 {
        self.0 & !(self.0 << 1)
    }

    #[inline]
    pub fn back_face_mask(&self) -> u64 {
        self.0 & !(self.0 >> 1)
    }
}

mod transpose {
    use crate::{
        util::bitmatrix_transpose::{
            BitMatrix,
            slice_get_mut_2,
        },
        voxel::mesh::opacity_mask::OpacityMask,
    };

    impl BitMatrix for [OpacityMask] {
        #[inline]
        fn len(&self) -> usize {
            <[_]>::len(self)
        }

        #[inline]
        fn get_mut_2(&mut self, rows: [usize; 2]) -> [&mut u64; 2] {
            let [a, b] = slice_get_mut_2(self, rows);
            [&mut a.0, &mut b.0]
        }
    }

    pub trait View {
        fn index(&self, index: usize) -> usize;
    }

    #[derive(Clone, Copy, Debug)]
    pub struct RowView {
        pub y: u16,
    }

    impl View for RowView {
        #[inline]
        fn index(&self, index: usize) -> usize {
            usize::try_from(morton::encode::<[u16; 2]>([
                u16::try_from(index).unwrap(),
                self.y,
            ]))
            .unwrap()
        }
    }

    #[derive(Clone, Copy, Debug)]
    pub struct ColumnView {
        pub x: u16,
    }

    impl View for ColumnView {
        #[inline]
        fn index(&self, index: usize) -> usize {
            usize::try_from(morton::encode::<[u16; 2]>([
                self.x,
                u16::try_from(index).unwrap(),
            ]))
            .unwrap()
        }
    }

    #[derive(Debug)]
    pub struct OpacityMaskView<'a, V> {
        pub mask: &'a mut [u64],
        pub side_length: usize,
        pub view: V,
    }

    impl<'a, V> BitMatrix for OpacityMaskView<'a, V>
    where
        V: View,
    {
        #[inline]
        fn len(&self) -> usize {
            self.side_length
        }

        #[inline]
        fn get_mut_2(&mut self, rows: [usize; 2]) -> [&mut u64; 2] {
            let indices = rows.map(|row| self.view.index(row));
            slice_get_mut_2(self.mask, indices)
        }
    }
}
