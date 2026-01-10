pub mod block_face;
pub mod flat;
pub mod greedy_quads;
pub mod svo;

use std::fmt::Debug;

use bevy_ecs::system::SystemParam;

use crate::{
    render::texture_atlas::AtlasId,
    voxel::block_face::BlockFace,
};

pub trait Voxel: Clone + Debug + Send + Sync + 'static {
    type Data: SystemParam;

    fn texture<'w, 's>(
        &self,
        face: BlockFace,
        data: &<Self::Data as SystemParam>::Item<'w, 's>,
    ) -> Option<AtlasId>;

    fn is_opaque<'w, 's>(&self, data: &<Self::Data as SystemParam>::Item<'w, 's>) -> bool;

    fn can_merge<'w, 's>(
        &self,
        other: &Self,
        data: &<Self::Data as SystemParam>::Item<'w, 's>,
    ) -> bool;
}
