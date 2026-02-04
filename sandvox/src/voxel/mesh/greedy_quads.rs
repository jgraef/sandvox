use nalgebra::{
    Point2,
    Point3,
};

use crate::{
    render::mesh::MeshBuilder,
    util::{
        bitmask,
        bitmatrix_transpose::BitMatrix,
    },
    voxel::{
        BlockFace,
        Voxel,
        VoxelData,
        chunk::{
            Chunk,
            ChunkShape,
        },
        mesh::{
            ChunkMesher,
            UnorientedQuad,
            opacity_mask::OpacityMasks,
        },
    },
};

#[derive(Debug)]
pub struct GreedyMesher<V> {
    opacity: OpacityMasks,
    mesh_face_buffer: MeshFaceBuffer<V>,
}

impl<V, S> ChunkMesher<V, S> for GreedyMesher<V>
where
    S: ChunkShape,
    V: Voxel,
{
    fn new(shape: &S) -> Self {
        Self {
            opacity: OpacityMasks::new(shape),
            mesh_face_buffer: MeshFaceBuffer::new(shape),
        }
    }

    #[profiling::function]
    fn mesh_chunk<D>(&mut self, chunk: &Chunk<V, S>, mesh_builder: &mut MeshBuilder, data: &D)
    where
        D: VoxelData<V>,
    {
        let chunk_size: u16 = chunk.shape().side_length().try_into().unwrap();

        self.opacity.fill(chunk, data);

        let mut mesh_quad = |quad: &GreedyQuad<V>, face| {
            if let Some(texture) = data.texture(&quad.voxel, face) {
                let mesh = quad.inner.mesh(face, texture);
                mesh_builder.push(mesh.vertices, mesh.faces);
            }
        };

        let xy_voxel = |xyz: Point3<u16>| &chunk[xyz];
        let zy_voxel = |zyx: Point3<u16>| &chunk[zyx.zyx()];
        let xz_voxel = |xzy: Point3<u16>| &chunk[xzy.xzy()];

        // XY front
        self.mesh_face_buffer.mesh_faces(
            chunk_size,
            xy_voxel,
            |xy| self.opacity.opacity_xy(xy).front_face_mask(),
            |quad| mesh_quad(&quad, BlockFace::Front),
            data,
        );

        // XY back
        self.mesh_face_buffer.mesh_faces(
            chunk_size,
            xy_voxel,
            |xy| self.opacity.opacity_xy(xy).back_face_mask(),
            |quad| mesh_quad(&quad, BlockFace::Back),
            data,
        );

        // ZY front (left)
        self.mesh_face_buffer.mesh_faces(
            chunk_size,
            zy_voxel,
            |zy| self.opacity.opacity_zy(zy).front_face_mask(),
            |quad| mesh_quad(&quad, BlockFace::Left),
            data,
        );

        // ZY back (right)
        self.mesh_face_buffer.mesh_faces(
            chunk_size,
            zy_voxel,
            |zy| self.opacity.opacity_zy(zy).back_face_mask(),
            |quad| mesh_quad(&quad, BlockFace::Right),
            data,
        );

        // XZ front (down)
        self.mesh_face_buffer.mesh_faces(
            chunk_size,
            xz_voxel,
            |xz| self.opacity.opacity_xz(xz).front_face_mask(),
            |quad| mesh_quad(&quad, BlockFace::Down),
            data,
        );

        // XY back (up)
        self.mesh_face_buffer.mesh_faces(
            chunk_size,
            xz_voxel,
            |xz| self.opacity.opacity_xz(xz).back_face_mask(),
            |quad| mesh_quad(&quad, BlockFace::Up),
            data,
        );
    }
}

#[derive(Debug)]
struct MeshFaceBuffer<V> {
    face_masks: Box<[u64]>,

    /// Quads that can still grow
    ///
    /// This is used while greedy meshing faces to keep track of quads that can
    /// still grow.
    active_quads: Vec<GreedyQuad<V>>,
}

impl<V> MeshFaceBuffer<V> {
    fn new<S>(shape: &S) -> Self
    where
        S: ChunkShape,
    {
        let side_length = shape.side_length();
        Self {
            face_masks: vec![0; side_length].into_boxed_slice(),
            active_quads: Vec::with_capacity(side_length),
        }
    }
}

impl<V> MeshFaceBuffer<V> {
    /// Documentation and variable names are for XY faces, but are
    /// representative for other directions as well.
    #[profiling::function]
    fn mesh_faces<'v, D>(
        &mut self,
        chunk_size: u16,
        get_voxel: impl Fn(Point3<u16>) -> &'v V,
        face_mask: impl Fn(Point2<u16>) -> u64,
        mut emit_quad: impl FnMut(GreedyQuad<V>),
        data: &D,
    ) where
        V: Voxel,
        D: VoxelData<V>,
    {
        for y in 0..chunk_size {
            // get XZ faces
            for x in 0..chunk_size {
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
                        data.can_merge(&quad.voxel, get_voxel(Point3::new(x, y, quad.inner.k)))
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
            for z in 0..chunk_size {
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
                        if !data.can_merge(&voxel, get_voxel(Point3::new(x0 + x, y, z))) {
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
