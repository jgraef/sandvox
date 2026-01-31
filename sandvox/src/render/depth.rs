use bevy_ecs::component::Component;

#[derive(Debug, Component)]
pub struct DepthPrepass {
    texture_view: wgpu::TextureView,
}

fn render_depth_prepass() {
    todo!();
}
