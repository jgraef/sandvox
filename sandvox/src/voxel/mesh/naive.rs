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
        for (point, voxel) in chunk.iter() {
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
}

#[derive(Debug)]
pub struct NaiveHullMesher;

impl<V, S> ChunkMesher<V, S> for NaiveHullMesher
where
    V: Voxel,
    S: ChunkShape,
{
    fn new(shape: &S) -> Self {
        let _ = shape;
        Self
    }

    fn mesh_chunk<D>(&mut self, chunk: &Chunk<V, S>, mesh_builder: &mut MeshBuilder, data: &D)
    where
        D: VoxelData<V>,
    {
        for (point, voxel) in chunk.iter() {
            let mut mesh_face = |point: Point3<u16>, face: BlockFace, ij: Point2<u16>, k: u16| {
                let is_visible = (point.coords.cast::<i16>() + face.neighbor())
                    .try_cast::<u16>()
                    .and_then(|point| chunk.get(point.into()))
                    .is_none_or(|neighbor| !data.is_opaque(neighbor));

                if is_visible && let Some(texture) = data.texture(voxel, face) {
                    let quad = UnorientedQuad {
                        ij0: ij,
                        ij1: ij + Vector2::repeat(1),
                        k,
                    };
                    let mesh = quad.mesh(face, texture);
                    mesh_builder.push(mesh.vertices, mesh.faces);
                }
            };

            mesh_face(point, BlockFace::Left, point.zy(), point.x);
            mesh_face(point, BlockFace::Right, point.zy(), point.x);
            mesh_face(point, BlockFace::Down, point.xz(), point.y);
            mesh_face(point, BlockFace::Up, point.xz(), point.y);
            mesh_face(point, BlockFace::Front, point.xy(), point.z);
            mesh_face(point, BlockFace::Back, point.xy(), point.z);
        }
    }
}
