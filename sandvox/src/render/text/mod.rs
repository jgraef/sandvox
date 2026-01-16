mod font_atlas;

use bevy_ecs::{
    component::Component,
    entity::Entity,
    query::{
        Changed,
        Or,
        Without,
    },
    resource::Resource,
    schedule::IntoScheduleConfigs,
    system::{
        Commands,
        Populated,
        ResMut,
    },
};
use color_eyre::eyre::Error;
use palette::{
    Srgba,
    WithAlpha,
};

use crate::{
    ecs::{
        plugin::{
            Plugin,
            WorldBuilder,
        },
        schedule,
    },
    render::text::font_atlas::create_font_atlas,
    wgpu::WgpuSystems,
};

#[derive(Clone, Copy, Debug, Default)]
pub struct TextPlugin;

impl Plugin for TextPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        builder
            .insert_resource(Fonts::new())
            .add_systems(
                schedule::Startup,
                create_font_atlas.after(WgpuSystems::CreateContext),
            )
            .add_systems(
                schedule::Render,
                // todo: before/after?
                (create_text_buffers, update_text_buffers),
            );

        Ok(())
    }
}

#[derive(Debug, Resource)]
struct Fonts {
    font_system: cosmic_text::FontSystem,
}

impl Fonts {
    fn new() -> Self {
        let font_db = cosmic_text::fontdb::Database::new();
        let font_system =
            cosmic_text::FontSystem::new_with_locale_and_db("en_US".to_owned(), font_db);

        for face in font_system.db().faces() {
            tracing::debug!(?face, "font");
        }

        Self { font_system }
    }

    fn buffer(&mut self, metrics: cosmic_text::Metrics) -> Buffer {
        let mut buffer = cosmic_text::Buffer::new(&mut self.font_system, metrics);
        buffer.set_size(&mut self.font_system, None, None);
        Buffer { buffer }
    }
}

#[derive(Clone, Debug, Component)]
pub struct Text {
    pub text: String,
}

#[derive(Clone, Debug, Component)]
struct Buffer {
    buffer: cosmic_text::Buffer,
}

#[derive(Clone, Copy, Debug, Component)]
pub struct FontMetrics {
    pub font_size: f32,
    pub line_height: f32,
}

impl From<FontMetrics> for cosmic_text::Metrics {
    fn from(value: FontMetrics) -> Self {
        Self {
            font_size: value.font_size,
            line_height: value.line_height,
        }
    }
}

#[derive(Clone, Debug, Component, PartialEq, Eq, derive_more::From)]
pub struct FontFamily(cosmic_text::FamilyOwned);

impl FontFamily {
    pub fn from_name(name: &str) -> Self {
        cosmic_text::FamilyOwned::Name(name.into()).into()
    }

    pub fn serif() -> Self {
        cosmic_text::FamilyOwned::Serif.into()
    }

    pub fn sans_serif() -> Self {
        cosmic_text::FamilyOwned::SansSerif.into()
    }

    pub fn cursive() -> Self {
        cosmic_text::FamilyOwned::Cursive.into()
    }

    pub fn fantasy() -> Self {
        cosmic_text::FamilyOwned::Fantasy.into()
    }

    pub fn monospace() -> Self {
        cosmic_text::FamilyOwned::Monospace.into()
    }
}

#[derive(Clone, Copy, Debug, Component, derive_more::From, derive_more::Into)]
pub struct FontColor {
    pub color: Srgba<f32>,
}

impl Default for FontColor {
    fn default() -> Self {
        Self {
            color: palette::named::BLACK.into_format().with_alpha(1.0),
        }
    }
}

fn create_text_buffers(
    mut fonts: ResMut<Fonts>,
    nodes: Populated<(Entity, &Text, &FontMetrics, &FontFamily), Without<Buffer>>,
    mut commands: Commands,
) {
    for (entity, text, metrics, font_family) in nodes.iter() {
        let mut buffer = fonts.buffer(metrics.clone().into());

        let attributes = cosmic_text::Attrs::new().family(font_family.0.as_family());
        buffer.buffer.set_text(
            &mut fonts.font_system,
            &text.text,
            &attributes,
            cosmic_text::Shaping::Basic,
            None,
        );

        commands.entity(entity).insert(buffer);
    }
}

// todo: handle font metrics change
fn update_text_buffers(
    mut fonts: ResMut<Fonts>,
    mut nodes: Populated<
        (&mut Buffer, &Text, &FontFamily),
        Or<(Changed<Text>, Changed<FontFamily>)>,
    >,
) {
    for (mut buffer, text, font_family) in nodes.iter_mut() {
        let attributes = cosmic_text::Attrs::new().family(font_family.0.as_family());
        buffer.buffer.set_text(
            &mut fonts.font_system,
            &text.text,
            &attributes,
            cosmic_text::Shaping::Basic,
            None,
        );
    }
}
