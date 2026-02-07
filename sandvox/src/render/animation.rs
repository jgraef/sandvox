use std::{
    collections::HashMap,
    hash::Hasher,
};

use bevy_ecs::{
    component::Component,
    entity::Entity,
    name::Name,
};
use color_eyre::eyre::Error;
use nalgebra::{
    Translation3,
    UnitQuaternion,
};
use seahash::SeaHasher;
use uuid::{
    Uuid,
    uuid,
};

use crate::ecs::plugin::{
    Plugin,
    WorldBuilder,
};

#[derive(Clone, Copy, Debug, Default)]
pub struct AnimationPlugin;

impl Plugin for AnimationPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        todo!()
    }
}

/// The [UUID namespace] of animation targets (e.g. bones).
///
/// [UUID namespace]: https://en.wikipedia.org/wiki/Universally_unique_identifier#Versions_3_and_5_(namespace_name-based)
pub static ANIMATION_TARGET_NAMESPACE: Uuid = uuid!("9431f294-901e-4826-9f15-61273d9375be");

#[derive(Clone, Debug, Default)]
pub struct AnimationClip {
    // todo
    duration: f32,
    channels: Vec<Channel>,
}

/// A component that links an animated entity to an entity containing an
/// [`AnimationPlayer`]. Typically used alongside the [`AnimationTargetId`]
/// component - the linked `AnimationPlayer` plays [`AnimationClip`] assets, and
/// the `AnimationTargetId` identifies which curves in the `AnimationClip` will
/// affect the target entity.
#[derive(Clone, Copy, Debug, Component)]
pub struct AnimatedBy(pub Entity);

/// A component that identifies which parts of an [`AnimationClip`] asset can
/// be applied to an entity. Typically used alongside the
/// [`AnimatedBy`] component.
///
/// `AnimationTargetId` is implemented as a [UUID]. When importing an armature
/// or an animation clip, asset loaders typically use the full path name from
/// the armature to the bone to generate these UUIDs. The ID is unique to the
/// full path name and based only on the names. So, for example, any imported
/// armature with a bone at the root named `Hips` will assign the same
/// [`AnimationTargetId`] to its root bone. Likewise, any imported animation
/// clip that animates a root bone named `Hips` will reference the same
/// [`AnimationTargetId`]. Any animation is playable on any armature as long as
/// the bone names match, which allows for easy animation retargeting.
///
/// Note that asset loaders generally use the *full* path name to generate the
/// [`AnimationTargetId`]. Thus a bone named `Chest` directly connected to a
/// bone named `Hips` will have a different ID from a bone named `Chest` that's
/// connected to a bone named `Stomach`.
///
/// [UUID]: https://en.wikipedia.org/wiki/Universally_unique_identifier
#[derive(Clone, Copy, Debug, Component, PartialEq, Eq, Hash)]
pub struct AnimationTargetId(pub Uuid);

impl AnimationTargetId {
    /// Creates a new [`AnimationTargetId`] by hashing a list of names.
    ///
    /// Typically, this will be the path from the animation root to the
    /// animation target (e.g. bone) that is to be animated.
    pub fn from_names<'a>(names: impl IntoIterator<Item = &'a Name>) -> Self {
        // we use 2 hashers to get a 128 bit hash for the uuid

        let mut h1 = SeaHasher::new();
        let mut h2 = SeaHasher::new();

        h1.write(ANIMATION_TARGET_NAMESPACE.as_bytes());
        h1.write_u8(1);
        h2.write(ANIMATION_TARGET_NAMESPACE.as_bytes());
        h1.write_u8(2);

        for name in names {
            h1.write_u64(name.len() as u64);
            h1.write(name.as_bytes());
            h2.write_u64(name.len() as u64);
            h2.write(name.as_bytes());
        }

        let mut h = [0u8; 16];
        h[0..8].copy_from_slice(&h1.finish().to_be_bytes());
        h[8..16].copy_from_slice(&h2.finish().to_be_bytes());

        let uuid = *uuid::Builder::from_bytes(h)
            .with_variant(uuid::Variant::RFC4122)
            .with_version(uuid::Version::Sha1)
            .as_uuid();

        Self(uuid)
    }

    /// Creates a new [`AnimationTargetId`] by hashing a single name.
    pub fn from_name(name: &Name) -> Self {
        Self::from_names([name])
    }
}

impl From<&Name> for AnimationTargetId {
    fn from(name: &Name) -> Self {
        AnimationTargetId::from_name(name)
    }
}

/// An animation that an [`AnimationPlayer`] is currently either playing or was
/// playing, but is presently paused.
///
/// A stopped animation is considered no longer active.
#[derive(Clone, Copy, Debug)]
pub struct ActiveAnimation {
    weight: f32,

    /// The speed with which this is animated.
    ///
    /// TODO: Does this just change the time step we do in this animation?
    speed: f32,

    /// Total time the animation has been played.
    ///
    /// Note: Time does not increase when the animation is paused or after it
    /// has completed.
    elapsed: f32,

    /// The timestamp inside of the animation clip.
    ///
    /// Note: This will always be in the range [0.0, animation clip duration]
    seek_time: f32,

    /// The `seek_time` of the previous tick, if any.
    previous_seek_time: Option<f32>,

    /// Number of times the animation has completed.
    /// If the animation is playing in reverse, this increments when the
    /// animation passes the start.
    completions: u32,

    /// `true` if the animation was completed at least once this tick.
    just_completed: bool,

    /// `true` if the animation is paused.
    paused: bool,
}

impl Default for ActiveAnimation {
    fn default() -> Self {
        Self {
            weight: 1.0,
            speed: 1.0,
            elapsed: 0.0,
            seek_time: 0.0,
            previous_seek_time: None,
            completions: 0,
            just_completed: false,
            paused: false,
        }
    }
}

/// Animation controls.
///
/// Automatically added to any root animations of a scene when it is
/// spawned.
#[derive(Clone, Debug, Default, Component)]
pub struct AnimationPlayer {
    active_animations: HashMap<AnimationNodeIndex, ActiveAnimation>,
}

// note: bevy uses an animation graph. we'll just refer to a single animation
// clip for now
pub type AnimationNodeIndex = u32;

// old

#[derive(Clone, Debug)]
pub enum Channel {
    Translation {
        key_frames: Vec<KeyFrame<Translation3<f32>>>,
    },
    Rotation {
        key_frames: Vec<KeyFrame<UnitQuaternion<f32>>>,
    },
}

#[derive(Clone, Debug)]
pub struct KeyFrame<V> {
    pub time: f32,
    pub value: V,
}
