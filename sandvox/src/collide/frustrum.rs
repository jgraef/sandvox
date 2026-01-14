use nalgebra::{
    Isometry3,
    Vector4,
};

use crate::collide::Aabb;

#[derive(Clone, Copy, Debug)]
pub struct Frustrum {
    // plane normals. they point into the frustrum. there is no far plane
    planes: [Vector4<f32>; 6],
}

impl Frustrum {
    pub fn new(horizontal_angle: f32, vertical_angle: f32, near: f32, far: f32) -> Self {
        let x_sin = horizontal_angle.sin();
        let x_cos = horizontal_angle.cos();
        let y_sin = vertical_angle.sin();
        let y_cos = vertical_angle.cos();

        let left = Vector4::new(x_cos, 0.0, x_sin, 0.0);
        let right = Vector4::new(-x_cos, 0.0, x_sin, 0.0);

        let bottom = Vector4::new(0.0, y_cos, y_sin, 0.0);
        let top = Vector4::new(0.0, -y_cos, y_sin, 0.0);

        let near = Vector4::new(0.0, 0.0, 1.0, near);
        let far = Vector4::new(0.0, 0.0, -1.0, far);

        Self {
            planes: [left, right, bottom, top, near, far],
        }
    }

    /// The isometry maps the AABBs' vertices to the frustrum's local space.
    ///
    /// E.g. If this is a camera frustrum and the AABBs are in world space,
    /// you'd want to pass the inverse camera transform.
    pub fn intersect_aabb(&self, isometry: &Isometry3<f32>, aabb: &Aabb) -> bool {
        //let mut outcodes_or = 0u8;
        let mut outcodes_and = u8::MAX;

        for vertex in aabb.vertices() {
            // map AABB vertex to frustrum coordinates
            let vertex = (isometry * vertex).to_homogeneous();

            // generate outcode
            let mut outcode = 0u8;

            for (i, plane) in self.planes.iter().enumerate() {
                // how far is the aabb vertex from the plane? (positive points inside frustrum)
                let distance = plane.dot(&vertex);

                if distance < 0.0 {
                    outcode |= 1 << i;
                }
            }

            //outcodes_or |= outcode;
            outcodes_and &= outcode;

            /*tracing::debug!(
                "  ({}, {}, {}): {:06b}",
                j & 1,
                (j >> 1) & 1,
                (j >> 2) & 1,
                outcode
            );*/
        }

        //tracing::debug!("|{:05b}, &{:05b}", outcodes_or, outcodes_and);
        //tracing::debug!("&{:06b}", outcodes_and);
        //tracing::debug!("");

        // all AABB vertices share an outzone -> trivial reject
        outcodes_and == 0
    }
}
