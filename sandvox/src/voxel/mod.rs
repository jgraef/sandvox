use bevy_ecs::system::SystemParam;

use crate::render::texture_atlas::AtlasId;

pub mod block_face;
pub mod flat;
pub mod svo;

pub trait Voxel {
    type SystemParam: SystemParam;

    fn texture<'w, 's>(
        &self,
        param: &mut <Self::SystemParam as SystemParam>::Item<'w, 's>,
    ) -> Option<AtlasId>;
}
