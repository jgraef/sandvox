use nalgebra::{
    Point2,
    Point3,
    UnitVector3,
    Vector2,
    Vector3,
};

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
