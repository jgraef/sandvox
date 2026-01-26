use nalgebra::Matrix4;

use crate::collide::Aabb;

#[derive(Clone, Copy, Debug)]
pub struct Frustrum {
    /// Matrix that projects the frustrum to a [-1, 1]^3 cube (e.g. a camera
    /// projection matrix).
    pub matrix: Matrix4<f32>,
}

impl Frustrum {
    pub fn intersect_aabb(&self, aabb: &Aabb) -> bool {
        let mut outcodes_and = u8::MAX;

        for vertex in aabb.vertices() {
            // map AABB vertex to frustrum space
            let vertex = self.matrix * vertex.to_homogeneous();

            // generate outcode
            let mut outcode = 0u8;

            if vertex.x < -vertex.w {
                outcode |= 1;
            }
            if vertex.x > vertex.w {
                outcode |= 2;
            }
            if vertex.y < -vertex.w {
                outcode |= 4;
            }
            if vertex.y > vertex.w {
                outcode |= 8;
            }
            if vertex.z < 0.0 {
                outcode |= 16;
            }
            if vertex.z > vertex.w {
                outcode |= 32;
            }

            outcodes_and &= outcode;
        }

        //tracing::debug!("&{:06b}", outcodes_and);

        // all AABB vertices share an outzone -> trivial reject
        outcodes_and == 0
    }
}
