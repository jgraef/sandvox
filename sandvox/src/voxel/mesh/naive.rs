use bevy_ecs::system::SystemParam;
use morton_encoding::morton_decode;
use nalgebra::{
    Point2,
    Point3,
    Vector2,
};

use crate::{
    render::mesh::MeshBuilder,
    voxel::{
        BlockFace,
        Voxel,
        flat::{
            CHUNK_NUM_VOXELS,
            FlatChunk,
        },
        mesh::{
            ChunkMesher,
            UnorientedQuad,
        },
    },
};

#[derive(Clone, Copy, Debug, Default)]
pub struct NaiveMesher;

impl<V> ChunkMesher<V> for NaiveMesher
where
    V: Voxel,
{
    fn mesh_chunk<'w, 's>(
        &mut self,
        chunk: &FlatChunk<V>,
        mesh_builder: &mut MeshBuilder,
        data: &<V::Data as SystemParam>::Item<'w, 's>,
    ) {
        naive_mesh(&chunk.voxels, mesh_builder, data);
    }
}

pub fn naive_mesh<'w, 's, V>(
    voxels: &[V; CHUNK_NUM_VOXELS],
    mesh_builder: &mut MeshBuilder,
    data: &<V::Data as SystemParam>::Item<'w, 's>,
) where
    V: Voxel,
{
    for (i, voxel) in voxels.iter().enumerate() {
        let point = Point3::from(morton_decode::<u16, 3>(i.try_into().unwrap()));

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
