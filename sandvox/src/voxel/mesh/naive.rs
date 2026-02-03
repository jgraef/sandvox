use nalgebra::{
    Point2,
    Vector2,
};

use crate::{
    render::mesh::MeshBuilder,
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
        },
    },
};

#[derive(Clone, Copy, Debug, Default)]
pub struct NaiveMesher;

impl<V, S> ChunkMesher<V, S> for NaiveMesher
where
    V: Voxel,
    S: ChunkShape,
{
    fn new(shape: &S) -> Self {
        let _ = shape;
        Default::default()
    }

    fn mesh_chunk<D>(&mut self, chunk: &Chunk<V, S>, mesh_builder: &mut MeshBuilder, data: &D)
    where
        D: VoxelData<V>,
    {
        naive_mesh(chunk, mesh_builder, data);
    }
}

pub fn naive_mesh<'w, 's, V, S, D>(voxels: &Chunk<V, S>, mesh_builder: &mut MeshBuilder, data: &D)
where
    V: Voxel,
    S: ChunkShape,
    D: VoxelData<V>,
{
    for (point, voxel) in voxels.iter() {
        let mut mesh_face = |face, ij: Point2<u16>, k: u16| {
            if let Some(texture) = data.texture(voxel, face) {
                let quad = UnorientedQuad {
                    ij0: ij,
                    ij1: ij + Vector2::repeat(1),
                    k,
                };
                let mesh = quad.mesh(face, texture);
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
