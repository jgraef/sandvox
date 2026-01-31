use bevy_ecs::component::Component;
use palette::LinSrgb;

#[derive(Clone, Copy, Debug, Component)]
pub struct DirectionalLight {
    pub color: LinSrgb<f32>,
    pub illuminance: f32,
}

#[derive(Clone, Copy, Debug, Default, Component)]
pub struct CastShadows;
