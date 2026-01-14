pub mod frustrum;

use nalgebra::{
    Point3,
    Vector3,
};

#[derive(Clone, Copy, Debug)]
pub struct Aabb {
    pub min: Point3<f32>,
    pub max: Point3<f32>,
}

impl Aabb {
    pub fn from_bounds(min: Point3<f32>, max: Point3<f32>) -> Self {
        let mut aabb = Self::from(min);
        aabb.push(max);
        aabb
    }

    pub fn from_size(point: Point3<f32>, size: Vector3<f32>) -> Self {
        let mut aabb = Self::from(point);
        aabb.push(point + size);
        aabb
    }

    pub fn push(&mut self, point: Point3<f32>) {
        self.min = self
            .min
            .coords
            .zip_map(&point.coords, |a, b| a.min(b))
            .into();

        self.max = self
            .max
            .coords
            .zip_map(&point.coords, |a, b| a.max(b))
            .into();
    }

    pub fn size(&self) -> Vector3<f32> {
        self.max - self.min
    }

    pub fn vertices(&self) -> [Point3<f32>; 8] {
        let select = |i, mask| {
            if i & mask == 0usize {
                &self.min
            }
            else {
                &self.max
            }
        };

        std::array::from_fn(|i| {
            Point3::new(select(i, 0b001).x, select(i, 0b010).y, select(i, 0b100).z)
        })
    }
}

impl From<Point3<f32>> for Aabb {
    fn from(value: Point3<f32>) -> Self {
        Self {
            min: value,
            max: value,
        }
    }
}

impl FromIterator<Point3<f32>> for Aabb {
    fn from_iter<T: IntoIterator<Item = Point3<f32>>>(iter: T) -> Self {
        let mut iter = iter.into_iter();
        let mut aabb = Aabb::from(iter.next().expect("Need atleast 1 point in interator"));
        for point in iter {
            aabb.push(point);
        }
        aabb
    }
}
