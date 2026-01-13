use nalgebra::{
    Point2,
    Vector2,
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
        },
    },
};

#[derive(Clone, Copy, Debug, Default)]
pub struct NaiveMesher;

impl<V, const CHUNK_SIZE: usize> ChunkMesher<V, CHUNK_SIZE> for NaiveMesher
where
    V: Voxel,
{
    fn mesh_chunk<'w, 's>(
        &mut self,
        chunk: &Chunk<V, CHUNK_SIZE>,
        mesh_builder: &mut MeshBuilder,
        data: &V::Data,
    ) {
        naive_mesh(chunk, mesh_builder, data);
    }
}

pub fn naive_mesh<'w, 's, V, const CHUNK_SIZE: usize>(
    voxels: &Chunk<V, CHUNK_SIZE>,
    mesh_builder: &mut MeshBuilder,
    data: &V::Data,
) where
    V: Voxel,
{
    for (point, voxel) in voxels.iter() {
        let mut mesh_face = |face, ij: Point2<u16>, k: u16| {
            if let Some(texture) = voxel.texture(face, data) {
                let quad = UnorientedQuad {
                    ij0: ij,
                    ij1: ij + Vector2::repeat(1),
                    k,
                };
                let mesh = quad.mesh(face, texture.into());
                mesh_builder.push(mesh.vertices, mesh.faces);
            }
        };

        mesh_face(BlockFace::Left, point.zy(), point.x);
        mesh_face(BlockFace::Right, point.zy(), point.x);
        mesh_face(BlockFace::Down, point.xz(), point.y);
        mesh_face(BlockFace::Up, point.xz(), point.y);
        mesh_face(BlockFace::Front, point.xy(), point.z);
        mesh_face(BlockFace::Back, point.xy(), point.z);
    }
}
