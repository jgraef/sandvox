use bevy_ecs::{
    resource::Resource,
    system::{
        Commands,
        Res,
    },
};
use cosmic_text::CacheKey;

use crate::{
    render::text::Fonts,
    wgpu::WgpuContext,
};

pub(super) fn create_font_atlas(wgpu: Res<WgpuContext>, mut commands: Commands) {
    let font_atlas = FontAtlas::new(&wgpu);
    commands.insert_resource(font_atlas);
}

#[derive(Debug, Resource)]
struct FontAtlas {
    //
}

impl FontAtlas {
    fn new(wgpu: &WgpuContext) -> Self {
        todo!();
    }

    fn get(&mut self, cache_key: CacheKey, fonts: &Fonts) -> Option<GlyphData> {
        todo!();
    }
}

#[derive(Clone, Copy, Debug)]
struct GlyphData {
    // todo
}
