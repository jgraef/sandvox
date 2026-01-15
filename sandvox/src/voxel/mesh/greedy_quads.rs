use morton_encoding::{
    morton_decode,
    morton_encode,
};
use nalgebra::{
    Point2,
    Point3,
};

use crate::{
    render::mesh::MeshBuilder,
    voxel::{
        BlockFace,
        Voxel,
        chunk::Chunk,
        mesh::{
            ChunkMesher,
            UnorientedQuad,
            greedy_quads::tranpose::{
                BitMatrix,
                ColumnView,
                OpacityMaskView,
                RowView,
            },
        },
    },
};

#[derive(Debug)]
pub struct GreedyMesher<V, const CHUNK_SIZE: usize> {
    opacity: OpacityMasks<CHUNK_SIZE>,
    mesh_face_buffer: MeshFaceBuffer<V, CHUNK_SIZE>,
}

impl<V, const CHUNK_SIZE: usize> Default for GreedyMesher<V, CHUNK_SIZE> {
    fn default() -> Self {
        Self {
            opacity: Default::default(),
            mesh_face_buffer: Default::default(),
        }
    }
}

impl<V, const CHUNK_SIZE: usize> ChunkMesher<V, CHUNK_SIZE> for GreedyMesher<V, CHUNK_SIZE>
where
    V: Voxel,
{
    fn mesh_chunk<'w, 's>(
        &mut self,
        chunk: &Chunk<V, CHUNK_SIZE>,
        mesh_builder: &mut MeshBuilder,
        data: &V::Data,
    ) {
        self.opacity.fill(chunk, data);

        let mut mesh_quad = |quad: &GreedyQuad<V>, face| {
            if let Some(texture) = quad.voxel.texture(face, data) {
                let mesh = quad.inner.mesh(face, texture.into());
                mesh_builder.push(mesh.vertices, mesh.faces);
            }
        };

        let xy_voxel = |xyz: Point3<u16>| &chunk[xyz];
        let zy_voxel = |zyx: Point3<u16>| &chunk[zyx.zyx()];
        let xz_voxel = |xzy: Point3<u16>| &chunk[xzy.xzy()];

        // XY front
        self.mesh_face_buffer.mesh_faces(
            xy_voxel,
            |xy| front_face_mask(self.opacity.opacity_xy(xy)),
            |quad| mesh_quad(&quad, BlockFace::Front),
            data,
        );

        // XY back
        self.mesh_face_buffer.mesh_faces(
            xy_voxel,
            |xy| back_face_mask(self.opacity.opacity_xy(xy)),
            |quad| mesh_quad(&quad, BlockFace::Back),
            data,
        );

        // ZY front (left)
        self.mesh_face_buffer.mesh_faces(
            zy_voxel,
            |zy| front_face_mask(self.opacity.opacity_zy(zy)),
            |quad| mesh_quad(&quad, BlockFace::Left),
            data,
        );

        // ZY back (right)
        self.mesh_face_buffer.mesh_faces(
            zy_voxel,
            |zy| back_face_mask(self.opacity.opacity_zy(zy)),
            |quad| mesh_quad(&quad, BlockFace::Right),
            data,
        );

        // XZ front (down)
        self.mesh_face_buffer.mesh_faces(
            xz_voxel,
            |xz| front_face_mask(self.opacity.opacity_xz(xz)),
            |quad| mesh_quad(&quad, BlockFace::Down),
            data,
        );

        // XY back (up)
        self.mesh_face_buffer.mesh_faces(
            xz_voxel,
            |xz| back_face_mask(self.opacity.opacity_xz(xz)),
            |quad| mesh_quad(&quad, BlockFace::Up),
            data,
        );
    }
}

#[derive(Debug)]
struct MeshFaceBuffer<V, const CHUNK_SIZE: usize> {
    face_masks: Box<[u64]>,

    /// Quads that can still grow
    ///
    /// This is used while greedy meshing faces to keep track of quads that can
    /// still grow.
    active_quads: Vec<GreedyQuad<V>>,
}

impl<V, const CHUNK_SIZE: usize> Default for MeshFaceBuffer<V, CHUNK_SIZE> {
    fn default() -> Self {
        Self {
            face_masks: vec![0; CHUNK_SIZE].into_boxed_slice(),
            active_quads: Vec::with_capacity(CHUNK_SIZE),
        }
    }
}

impl<V, const CHUNK_SIZE: usize> MeshFaceBuffer<V, CHUNK_SIZE> {
    /// Documentation and variable names are for XY faces, but are
    /// representative for other directions as well.
    fn mesh_faces<'v>(
        &mut self,
        get_voxel: impl Fn(Point3<u16>) -> &'v V,
        face_mask: impl Fn(Point2<u16>) -> u64,
        mut emit_quad: impl FnMut(GreedyQuad<V>),
        data: &V::Data,
    ) where
        V: Voxel,
    {
        for y in 0..CHUNK_SIZE as u16 {
            // get XZ faces
            for x in 0..CHUNK_SIZE as u16 {
                self.face_masks[x as usize] = face_mask(Point2::new(x, y));
            }

            // transpose to ZX
            (*self.face_masks).transpose();

            // try to grow quads
            let mut quad_index = 0;
            while let Some(quad) = self.active_quads.get_mut(quad_index) {
                debug_assert_eq!(quad.inner.ij1.y, y);

                let face_mask = &mut self.face_masks[quad.inner.k as usize];
                let mut quad_grown = false;

                // check if this quad can grow vertically to the current row.
                // this doesn't yet take into account different block types, only if there are
                // faces to be generated.
                if quad.mask & *face_mask == quad.mask {
                    // check if we can actually merge these voxels
                    let can_merge = (quad.inner.ij0.x..quad.inner.ij1.x).all(|x| {
                        quad.voxel
                            .can_merge(get_voxel(Point3::new(x, y, quad.inner.k)), data)
                    });

                    if can_merge {
                        // mark faces as meshed
                        *face_mask &= !quad.mask;

                        // grow quad
                        quad.inner.ij1.y += 1;
                        quad_grown = true;
                    }
                }

                if quad_grown {
                    // quad was grown, continue to next active quad
                    quad_index += 1;
                }
                else {
                    // the quad wasn't grown to contain voxels in this row, so it can't ever grow
                    // again. thus we remove it from the active quads list and mesh it.
                    //
                    // note: don't increment quad_index here, as we just swapped another active quad
                    // in this place.
                    let quad = self.active_quads.swap_remove(quad_index);
                    emit_quad(quad);
                }
            }

            // create active quads for any faces that hasn't been meshed yet
            for z in 0..CHUNK_SIZE as u16 {
                let mut face_mask = self.face_masks[z as usize];

                // keeps track of how many voxels in the row have already been processed. the
                // face mask has also been shifted by this amount.
                let mut x0 = 0;

                while face_mask != 0 {
                    let first_face = face_mask.trailing_zeros() as u16;
                    face_mask >>= first_face;
                    x0 += first_face;

                    let mut num_faces = face_mask.trailing_ones() as u16;

                    // there are `num_faces` faces starting at `x0`, but they might not
                    // all be mergable.

                    // get first voxel in this range
                    let voxel = get_voxel(Point3::new(x0, y, z)).clone();

                    // find first voxel in this range that can't be merged (relative to x0).
                    // if we find one, this relative position is the actual number of faces we
                    // can merge
                    for x in 1..num_faces {
                        if !voxel.can_merge(get_voxel(Point3::new(x0 + x, y, z)), data) {
                            num_faces = x;
                            break;
                        }
                    }

                    face_mask >>= num_faces;
                    let x1 = x0 + num_faces;

                    // make mask
                    //          n    x1   x0   0
                    //          |    |     |   |
                    // a     =  0---01---------1
                    // b     =  0---------01---1
                    // a ^ b =  0---01----10---0
                    let mask = bitmask(x1 as usize) ^ bitmask(x0 as usize);

                    let quad = GreedyQuad {
                        voxel,
                        inner: UnorientedQuad {
                            ij0: Point2::new(x0, y),
                            ij1: Point2::new(x1, y + 1),
                            k: z,
                        },
                        mask,
                    };
                    self.active_quads.push(quad);

                    x0 = x1;
                }
            }
        }

        // we're done here, emit all quads that are active.
        for quad in self.active_quads.drain(..) {
            emit_quad(quad);
        }
    }
}

/// # Note
///
/// Comments and field names are choosen representively for a quad with
/// [`BlockFace::Front`]
#[derive(Clone, Copy, Debug)]
struct GreedyQuad<V> {
    voxel: V,
    inner: UnorientedQuad,
    /// which voxels are covered by this quad in X direction
    mask: u64,
}

/// Opacity masks for 3 axis: XY, ZY, XZ
#[derive(Debug)]
struct OpacityMasks<const CHUNK_SIZE: usize> {
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

impl<const CHUNK_SIZE: usize> Default for OpacityMasks<CHUNK_SIZE> {
    fn default() -> Self {
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

        Self {
            xy: vec![0; CHUNK_SIZE * CHUNK_SIZE].into_boxed_slice(),
            zy: vec![0; CHUNK_SIZE * CHUNK_SIZE].into_boxed_slice(),
            xz: vec![0; CHUNK_SIZE * CHUNK_SIZE].into_boxed_slice(),
        }
    }
}

impl<const CHUNK_SIZE: usize> OpacityMasks<CHUNK_SIZE> {
    fn fill<V>(&mut self, chunk: &Chunk<V, CHUNK_SIZE>, data: &V::Data)
    where
        V: Voxel,
    {
        // fill XY opacity matrix
        for i in 0..(CHUNK_SIZE * CHUNK_SIZE) {
            let [x, y] = morton_decode::<u16, 2>(i.try_into().unwrap());
            let mut mask_i = 0;
            for z in 0..CHUNK_SIZE as u16 {
                if chunk[Point3::new(x, y, z)].is_opaque(data) {
                    mask_i |= 1 << z;
                }
            }
            self.xy[i] = mask_i;
        }

        // flip X and Z
        self.zy.copy_from_slice(&self.xy);
        for y in 0..CHUNK_SIZE as u16 {
            OpacityMaskView {
                mask: &mut self.zy,
                side_length: CHUNK_SIZE,
                view: RowView { y },
            }
            .transpose();
        }

        // flip Y and Z
        self.xz.copy_from_slice(&self.xy);
        for x in 0..CHUNK_SIZE as u16 {
            OpacityMaskView {
                mask: &mut self.xz,
                side_length: CHUNK_SIZE,
                view: ColumnView { x },
            }
            .transpose();
        }
    }

    #[inline(always)]
    fn opacity_xy(&self, xy: Point2<u16>) -> u64 {
        let i: usize = morton_encode(xy.into()).try_into().unwrap();
        self.xy[i]
    }

    #[inline(always)]
    fn opacity_zy(&self, zy: Point2<u16>) -> u64 {
        let i: usize = morton_encode(zy.into()).try_into().unwrap();
        self.zy[i]
    }

    #[inline(always)]
    fn opacity_xz(&self, xz: Point2<u16>) -> u64 {
        let i: usize = morton_encode(xz.into()).try_into().unwrap();
        self.xz[i]
    }
}

#[inline(always)]
fn front_face_mask(opacity_mask: u64) -> u64 {
    opacity_mask & !(opacity_mask << 1)
}

#[inline(always)]
fn back_face_mask(opacity_mask: u64) -> u64 {
    opacity_mask & !(opacity_mask >> 1)
}

mod tranpose {
    // stuff to transpose bit matrices

    use morton_encoding::morton_encode;

    use crate::voxel::mesh::greedy_quads::bitmask;

    pub trait View {
        fn index(&self, index: usize) -> usize;
    }

    #[derive(Clone, Copy, Debug)]
    pub struct RowView {
        pub y: u16,
    }

    impl View for RowView {
        fn index(&self, index: usize) -> usize {
            usize::try_from(morton_encode([u16::try_from(index).unwrap(), self.y])).unwrap()
        }
    }

    #[derive(Clone, Copy, Debug)]
    pub struct ColumnView {
        pub x: u16,
    }

    impl View for ColumnView {
        fn index(&self, index: usize) -> usize {
            usize::try_from(morton_encode([self.x, u16::try_from(index).unwrap()])).unwrap()
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
        fn len(&self) -> usize {
            self.side_length
        }

        fn get_mut_2(&mut self, rows: [usize; 2]) -> [&mut u64; 2] {
            let indices = rows.map(|row| self.view.index(row));
            slice_get_mut_2(self.mask, indices)
        }
    }

    pub trait BitMatrix {
        fn len(&self) -> usize;
        fn get_mut_2(&mut self, rows: [usize; 2]) -> [&mut u64; 2];

        /// Taken from [`dsnet/matrix-transpose`][1]
        ///
        /// [1]: https://github.com/dsnet/matrix-transpose
        fn transpose(&mut self) {
            //let mut swap_width = 64;
            //let mut swap_mask = u64::MAX;

            let mut swap_width = self.len();
            if swap_width < 2 {
                return;
            }
            assert!(swap_width.is_power_of_two());

            let mut swap_mask = bitmask(swap_width);

            let mut outer_count = 1;

            while swap_width != 1 {
                swap_width >>= 1;
                swap_mask = swap_mask ^ (swap_mask >> swap_width);

                for outer in 0..outer_count {
                    for inner in 0..swap_width {
                        let inner_offset = inner + outer * swap_width * 2;
                        let [x, y] = self.get_mut_2([inner_offset, inner_offset + swap_width]);

                        *x = ((*y << swap_width) & swap_mask) ^ *x;
                        *y = ((*x & swap_mask) >> swap_width) ^ *y;
                        *x = ((*y << swap_width) & swap_mask) ^ *x;
                    }
                }

                outer_count <<= 1;
            }
        }
    }

    impl BitMatrix for [u64] {
        fn len(&self) -> usize {
            <[_]>::len(self)
        }

        fn get_mut_2(&mut self, rows: [usize; 2]) -> [&mut u64; 2] {
            slice_get_mut_2(self, rows)
        }
    }

    fn slice_get_mut_2<T>(slice: &mut [T], [i, j]: [usize; 2]) -> [&mut T; 2] {
        if i < j {
            let (left, right) = slice.split_at_mut(j);
            [&mut left[i], &mut right[0]]
        }
        else if j < i {
            let (left, right) = slice.split_at_mut(i);
            [&mut right[0], &mut left[j]]
        }
        else {
            panic!("Both indices can't be equal: {i} != {j}");
        }
    }
}

fn bitmask(n: usize) -> u64 {
    assert!(n <= 64);
    if n < 64 {
        (1 << n) - 1
    }
    else if n == 64 {
        u64::MAX
    }
    else {
        unreachable!();
    }
}
