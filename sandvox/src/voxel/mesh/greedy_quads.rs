use bevy_ecs::system::SystemParam;
use morton_encoding::{
    morton_decode,
    morton_encode,
};
use nalgebra::Point2;

use crate::{
    render::mesh::MeshBuilder,
    voxel::{
        BlockFace,
        Voxel,
        flat::{
            CHUNK_NUM_VOXELS,
            CHUNK_SIDE_LENGTH,
            FlatChunk,
        },
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

const LAYER_SIZE: usize = (CHUNK_SIDE_LENGTH * CHUNK_SIDE_LENGTH) as usize;

#[derive(Debug)]
pub struct GreedyMesher<V> {
    opacity: OpacityMasks,

    /// Quads that can still grow
    ///
    /// This is used while greedy meshing faces to keep track of quads that can
    /// still grow.
    active_quads: Vec<GreedyQuad<V>>,
}

impl<V> Default for GreedyMesher<V> {
    fn default() -> Self {
        Self {
            opacity: Default::default(),
            active_quads: Default::default(),
        }
    }
}

impl<V> ChunkMesher<V> for GreedyMesher<V>
where
    V: Voxel,
{
    fn mesh_chunk<'w, 's>(
        &mut self,
        chunk: &FlatChunk<V>,
        mesh_builder: &mut MeshBuilder,
        data: &<V::Data as SystemParam>::Item<'w, 's>,
    ) {
        self.mesh(&chunk.voxels, mesh_builder, data);
    }
}

impl<V> GreedyMesher<V>
where
    V: Voxel,
{
    pub fn mesh<'w, 's>(
        &mut self,
        chunk: &[V; CHUNK_NUM_VOXELS],
        mesh_builder: &mut MeshBuilder,
        data: &<V::Data as SystemParam>::Item<'w, 's>,
    ) {
        self.opacity.fill(chunk, data);

        let mut mesh_quad = |quad: &GreedyQuad<V>, face| {
            if let Some(texture) = quad.voxel.texture(face, data) {
                let mesh = quad.inner.mesh(face, texture.into());
                mesh_builder.push(mesh.vertices, mesh.faces);
            }
        };

        let xy_voxel = |xyz| &chunk[morton_encode(xyz) as usize];
        let zy_voxel = |[z, y, x]: [u16; 3]| &chunk[morton_encode([x, y, z]) as usize];
        let xz_voxel = |[x, z, y]: [u16; 3]| &chunk[morton_encode([x, y, z]) as usize];

        // XY front
        mesh_faces(
            xy_voxel,
            |xy| front_face_mask(self.opacity.opacity_xy(xy)),
            &mut self.active_quads,
            |quad| mesh_quad(&quad, BlockFace::Front),
            data,
        );

        // XY back
        mesh_faces(
            xy_voxel,
            |xy| back_face_mask(self.opacity.opacity_xy(xy)),
            &mut self.active_quads,
            |quad| mesh_quad(&quad, BlockFace::Back),
            data,
        );

        // ZY front (left)
        mesh_faces(
            zy_voxel,
            |zy| front_face_mask(self.opacity.opacity_zy(zy)),
            &mut self.active_quads,
            |quad| mesh_quad(&quad, BlockFace::Left),
            data,
        );

        // ZY back (right)
        mesh_faces(
            zy_voxel,
            |zy| back_face_mask(self.opacity.opacity_zy(zy)),
            &mut self.active_quads,
            |quad| mesh_quad(&quad, BlockFace::Right),
            data,
        );

        // XZ front (down)
        mesh_faces(
            xz_voxel,
            |xz| front_face_mask(self.opacity.opacity_xz(xz)),
            &mut self.active_quads,
            |quad| mesh_quad(&quad, BlockFace::Down),
            data,
        );

        // XY back (up)
        mesh_faces(
            xz_voxel,
            |xz| back_face_mask(self.opacity.opacity_xz(xz)),
            &mut self.active_quads,
            |quad| mesh_quad(&quad, BlockFace::Up),
            data,
        );
    }
}

/// Documentation and variable names are for XY faces, but are representative
/// for other directions as well.
fn mesh_faces<'w, 's, 'v, V>(
    get_voxel: impl Fn([u16; 3]) -> &'v V,
    face_mask: impl Fn([u16; 2]) -> u64,
    active_quads: &mut Vec<GreedyQuad<V>>,
    mut emit_quad: impl FnMut(GreedyQuad<V>),
    data: &<V::Data as SystemParam>::Item<'w, 's>,
) where
    V: Voxel,
{
    for y in 0..CHUNK_SIDE_LENGTH {
        // get XZ faces
        let mut face_masks = [0u64; CHUNK_SIDE_LENGTH as usize];
        //let mut back_faces = [0u64; CHUNK_SIDE_LENGTH as usize];

        for x in 0..CHUNK_SIDE_LENGTH {
            face_masks[x as usize] = face_mask([x, y]);
        }

        // transpose to ZX
        face_masks.as_mut_slice().transpose();

        // try to grow quads
        let mut quad_index = 0;
        while let Some(quad) = active_quads.get_mut(quad_index) {
            debug_assert_eq!(quad.inner.ij1.y, y);

            let face_mask = &mut face_masks[quad.inner.k as usize];
            let mut quad_grown = false;

            // check if this quad can grow vertically to the current row.
            // this doesn't yet take into account different block types, only if there are
            // faces to be generated.
            if quad.mask & *face_mask == quad.mask {
                // check if we can actually merge these voxels
                let can_merge = (quad.inner.ij0.x..quad.inner.ij1.x)
                    .all(|x| quad.voxel.can_merge(get_voxel([x, y, quad.inner.k]), data));

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
                let quad = active_quads.swap_remove(quad_index);
                emit_quad(quad);
            }
        }

        // create active quads for any faces that hasn't been meshed yet
        for z in 0..CHUNK_SIDE_LENGTH {
            let mut face_mask = face_masks[z as usize];

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
                let voxel = get_voxel([x0, y, z]).clone();

                // find first voxel in this range that can't be merged (relative to x0).
                // if we find one, this relative position is the actual number of faces we
                // can merge
                for x in 1..num_faces {
                    if !voxel.can_merge(get_voxel([x0 + x, y, z]), data) {
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
                active_quads.push(quad);

                x0 = x1;
            }
        }
    }

    // we're done here, emit all quads that are active.
    for quad in active_quads.drain(..) {
        emit_quad(quad);
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
struct OpacityMasks {
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
    masks: Box<[[u64; LAYER_SIZE]; 3]>,
}

impl Default for OpacityMasks {
    /// This is rather large (288 KiB for 64^3 chunks) so it is heap-allocated.
    ///
    /// The implementation of [`Default` for `Box`][1] seems to not construct
    /// the value on the stack and then move it, but initialize it on the heap
    /// directly - which is desired.
    ///
    /// Unfortunately for some reason `Default` is not implemented for large
    /// arrays, so we can't use that. Let's just hope this is optimized.
    ///
    /// [1]: https://doc.rust-lang.org/src/alloc/boxed.rs.html#1694
    fn default() -> Self {
        Self {
            masks: Box::new(std::array::from_fn(|_| std::array::from_fn(|_| 0))),
        }
    }
}

impl OpacityMasks {
    fn fill<'w, 's, V>(
        &mut self,
        chunk: &[V; CHUNK_NUM_VOXELS],
        data: &<V::Data as SystemParam>::Item<'w, 's>,
    ) where
        V: Voxel,
    {
        let [opacity_xy, opacity_zy, opacity_xz] = &mut *self.masks;

        // fill XY opacity matrix
        for i in 0..LAYER_SIZE {
            let [x, y] = morton_decode::<u16, 2>(i.try_into().unwrap());
            let mut mask_i = 0;
            for z in 0..CHUNK_SIDE_LENGTH {
                let j = morton_encode([x, y, z]);
                if chunk[j as usize].is_opaque(data) {
                    mask_i |= 1 << z;
                }
            }
            opacity_xy[i] = mask_i;
        }

        // flip X and Z
        opacity_zy.copy_from_slice(opacity_xy.as_slice());
        for y in 0..CHUNK_SIDE_LENGTH {
            OpacityMaskView {
                mask: opacity_zy,
                view: RowView { y },
            }
            .transpose();
        }

        // flip Y and Z
        opacity_xz.copy_from_slice(opacity_xy.as_slice());
        for x in 0..CHUNK_SIDE_LENGTH {
            OpacityMaskView {
                mask: opacity_xz,
                view: ColumnView { x },
            }
            .transpose();
        }
    }

    #[inline(always)]
    fn opacity_xy(&self, xy: [u16; 2]) -> u64 {
        let i: usize = morton_encode(xy).try_into().unwrap();
        self.masks[0][i]
    }

    #[inline(always)]
    fn opacity_zy(&self, zy: [u16; 2]) -> u64 {
        let i: usize = morton_encode(zy).try_into().unwrap();
        self.masks[1][i]
    }

    #[inline(always)]
    fn opacity_xz(&self, xz: [u16; 2]) -> u64 {
        let i: usize = morton_encode(xz).try_into().unwrap();
        self.masks[2][i]
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

    use crate::voxel::{
        flat::CHUNK_SIDE_LENGTH,
        mesh::greedy_quads::{
            LAYER_SIZE,
            bitmask,
        },
    };

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
        pub mask: &'a mut [u64; LAYER_SIZE],
        pub view: V,
    }

    impl<'a, V> BitMatrix for OpacityMaskView<'a, V>
    where
        V: View,
    {
        fn len(&self) -> usize {
            CHUNK_SIDE_LENGTH as usize
        }

        fn get_mut_2(&mut self, rows: [usize; 2]) -> [&mut u64; 2] {
            let indices = rows.map(|row| self.view.index(row));
            slice_get_mut_2(self.mask.as_mut_slice(), indices)
        }
    }

    pub trait BitMatrix {
        fn len(&self) -> usize;
        fn get_mut_2(&mut self, rows: [usize; 2]) -> [&mut u64; 2];

        /// https://github.com/dsnet/matrix-transpose
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

    impl BitMatrix for &mut [u64] {
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
