pub mod greedy_quads;
pub mod naive;

use bevy_ecs::system::SystemParam;
use nalgebra::{
    Point2,
    Point3,
    Vector3,
    Vector4,
};

use crate::{
    render::mesh::{
        MeshBuilder,
        Vertex,
    },
    voxel::{
        BlockFace,
        Voxel,
        flat::FlatChunk,
    },
};

pub trait ChunkMesher<V>: Send + Sync + Default + 'static
where
    V: Voxel,
{
    fn mesh_chunk<'w, 's>(
        &mut self,
        chunk: &FlatChunk<V>,
        mesh_builder: &mut MeshBuilder,
        data: &<V::Data as SystemParam>::Item<'w, 's>,
    );
}

#[derive(Clone, Copy, Debug)]
pub struct UnorientedQuad {
    pub ij0: Point2<u16>,
    pub ij1: Point2<u16>,
    pub k: u16,
}

impl UnorientedQuad {
    #[inline(always)]
    fn xy_vertices(&self) -> [Point3<u16>; 4] {
        [
            [self.ij0.x, self.ij0.y, self.k],
            [self.ij1.x, self.ij0.y, self.k],
            [self.ij1.x, self.ij1.y, self.k],
            [self.ij0.x, self.ij1.y, self.k],
        ]
        .map(Into::into)
    }

    #[inline(always)]
    fn zy_vertices(&self) -> [Point3<u16>; 4] {
        [
            [self.k, self.ij1.y, self.ij0.x],
            [self.k, self.ij1.y, self.ij1.x],
            [self.k, self.ij0.y, self.ij1.x],
            [self.k, self.ij0.y, self.ij0.x],
        ]
        .map(Into::into)
    }

    #[inline(always)]
    fn xz_vertices(&self) -> [Point3<u16>; 4] {
        [
            [self.ij0.x, self.k, self.ij1.y],
            [self.ij1.x, self.k, self.ij1.y],
            [self.ij1.x, self.k, self.ij0.y],
            [self.ij0.x, self.k, self.ij0.y],
        ]
        .map(Into::into)
    }

    #[inline(always)]
    fn uvs(&self) -> [Point2<u16>; 4] {
        let dx = self.ij1.x - self.ij0.x;
        let dy = self.ij1.y - self.ij0.y;

        //[[0, 0], [dx, 0], [dx, dy], [0, dy]]

        // pretty sure this is the right way. y is flipped
        [[0, dy], [dx, dy], [dx, 0], [0, 0]].map(Into::into)
    }

    pub fn mesh(&self, face: BlockFace, texture_id: u32) -> QuadMesh {
        let (vertices, normal, indices, offset) = match face {
            BlockFace::Left => {
                (
                    self.zy_vertices(),
                    -Vector4::x(),
                    FRONT_INDICES,
                    Vector3::zeros(),
                )
            }
            BlockFace::Right => (self.zy_vertices(), Vector4::x(), BACK_INDICES, Vector3::x()),
            BlockFace::Down => {
                (
                    self.xz_vertices(),
                    -Vector4::y(),
                    FRONT_INDICES,
                    Vector3::zeros(),
                )
            }
            BlockFace::Up => (self.xz_vertices(), Vector4::y(), BACK_INDICES, Vector3::y()),
            BlockFace::Front => {
                (
                    self.xy_vertices(),
                    -Vector4::z(),
                    FRONT_INDICES,
                    Vector3::zeros(),
                )
            }
            BlockFace::Back => (self.xy_vertices(), Vector4::z(), BACK_INDICES, Vector3::z()),
        };

        let uvs = self.uvs();

        let vertices = std::array::from_fn::<_, 4, _>(|i| {
            Vertex {
                position: (vertices[i].cast() + offset).to_homogeneous(),
                normal,
                uv: Point2::from(uvs[i]).cast(),
                texture_id,
            }
        });

        QuadMesh {
            vertices,
            faces: indices,
        }
    }
}

pub const FRONT_INDICES: [[u32; 3]; 2] = [[0, 1, 2], [0, 2, 3]];
pub const BACK_INDICES: [[u32; 3]; 2] = [[2, 1, 0], [3, 2, 0]];

#[derive(Clone, Copy, Debug)]
pub struct QuadMesh {
    pub vertices: [Vertex; 4],
    pub faces: [[u32; 3]; 2],
}
