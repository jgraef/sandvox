use std::ops::Range;

use bytemuck::{
    Pod,
    Zeroable,
};
use nalgebra::{
    Point2,
    Point3,
    UnitVector3,
    Vector2,
    Vector3,
    Vector4,
};
use wgpu::util::DeviceExt;

use crate::wgpu::WgpuContext;

#[derive(Clone, Debug, Default)]
pub struct MeshBuilder {
    vertices: Vec<Vertex>,
    faces: Vec<[u32; 3]>,
}

impl MeshBuilder {
    pub fn clear(&mut self) {
        self.vertices.clear();
        self.faces.clear();
    }

    pub fn push_quad(&mut self, anchor: Point3<u16>, size: Vector2<u16>, face: BlockFace) {
        let base_index: u32 = self.vertices.len().try_into().unwrap();

        let normal = face.normal().cast::<f32>().to_homogeneous();
        let positions = face.vertices(size);
        let uvs = face.uvs(size);
        let faces = face.faces();

        self.vertices.extend((0..4).map(|i| {
            Vertex {
                position: (anchor + positions[i].coords)
                    .cast::<f32>()
                    .to_homogeneous(),
                normal,
                uv: uvs[i].coords.cast::<f32>().into(),
            }
        }));

        self.faces
            .extend(faces.map(|face| face.map(|index| u32::from(index) + base_index)));
    }

    pub fn finish(&self, wgpu: &WgpuContext, label: &str) -> Option<Mesh> {
        if self.faces.is_empty() {
            None
        }
        else {
            assert!(!self.vertices.is_empty());

            let vertex_buffer = wgpu
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(&format!("{label} vertices")),
                    contents: bytemuck::cast_slice(&self.vertices),
                    usage: wgpu::BufferUsages::VERTEX,
                });

            let index_buffer = wgpu
                .device
                .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(&format!("{label} indices")),
                    contents: bytemuck::cast_slice(&self.faces),
                    usage: wgpu::BufferUsages::INDEX,
                });

            let num_indices = 3 * u32::try_from(self.faces.len()).unwrap();

            Some(Mesh {
                vertex_buffer,
                index_buffer,
                indices: 0..num_indices,
                base_vertex: 0,
            })
        }
    }
}

#[derive(Clone, Copy, Debug, Pod, Zeroable)]
#[repr(C)]
pub struct Vertex {
    position: Vector4<f32>,
    normal: Vector4<f32>,
    uv: Point2<f32>,
}

impl Vertex {
    pub const LAYOUT: wgpu::VertexBufferLayout<'static> = wgpu::VertexBufferLayout {
        array_stride: size_of::<Self>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &wgpu::vertex_attr_array![
            0 => Float32x4,
            1 => Float32x4,
            2 => Float32x2,
        ],
    };
}

#[derive(Clone, Debug)]
pub struct Mesh {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: wgpu::Buffer,
    pub indices: Range<u32>,
    pub base_vertex: i32,
}

impl Mesh {
    pub const INDEX_FORMAT: wgpu::IndexFormat = wgpu::IndexFormat::Uint32;

    pub fn draw(
        &self,
        render_pass: &mut wgpu::RenderPass,
        vertex_buffer_slot: u32,
        instances: Range<u32>,
    ) {
        render_pass.set_vertex_buffer(vertex_buffer_slot, self.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), Self::INDEX_FORMAT);
        render_pass.draw_indexed(self.indices.clone(), self.base_vertex, instances);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BlockFace {
    Left,
    Right,
    Up,
    Down,
    Front,
    Back,
}

impl BlockFace {
    pub const ALL: [Self; 6] = [
        Self::Left,
        Self::Right,
        Self::Up,
        Self::Down,
        Self::Front,
        Self::Back,
    ];

    pub fn normal(&self) -> UnitVector3<i8> {
        match self {
            BlockFace::Left => -Vector3::x_axis(),
            BlockFace::Right => Vector3::x_axis(),
            BlockFace::Up => -Vector3::y_axis(),
            BlockFace::Down => Vector3::y_axis(),
            BlockFace::Front => -Vector3::z_axis(),
            BlockFace::Back => Vector3::z_axis(),
        }
    }

    pub fn vertices(&self, size: Vector2<u16>) -> [Point3<u16>; 4] {
        match self {
            BlockFace::Left | BlockFace::Right => {
                [
                    [0, 0, 0],
                    [0, size.x, 0],
                    [0, size.x, size.y],
                    [0, 0, size.y],
                ]
            }
            BlockFace::Up | BlockFace::Down => {
                [
                    [0, 0, 0],
                    [size.x, 0, 0],
                    [size.x, 0, size.y],
                    [0, 0, size.y],
                ]
            }
            BlockFace::Front | BlockFace::Back => {
                [
                    [0, 0, 0],
                    [size.x, 0, 0],
                    [size.x, size.y, 0],
                    [0, size.y, 0],
                ]
            }
        }
        .map(Into::into)
    }

    pub fn uvs(&self, size: Vector2<u16>) -> [Point2<u16>; 4] {
        [[0, size.y], [size.x, size.y], [size.x, 0], [0, 0]].map(Into::into)
    }

    pub fn faces(&self) -> [[u8; 3]; 2] {
        match self {
            BlockFace::Left | BlockFace::Up | BlockFace::Front => [[0, 1, 2], [0, 2, 3]],
            BlockFace::Down | BlockFace::Right | BlockFace::Back => [[2, 1, 0], [3, 2, 0]],
        }
    }
}
