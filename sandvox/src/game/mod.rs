pub mod block_type;
pub mod camera_controller;
pub mod celestial;
pub mod file;
pub mod terrain;

use std::{
    fmt::Write,
    path::PathBuf,
    time::Duration,
};

use bevy_ecs::{
    component::Component,
    name::Name,
    query::{
        Changed,
        With,
    },
    resource::Resource,
    schedule::{
        IntoScheduleConfigs,
        SystemCondition,
        common_conditions::{
            any_with_component,
            resource_changed,
        },
    },
    system::{
        Commands,
        ParamSet,
        Populated,
        Query,
        Res,
        ResMut,
        Single,
    },
};
use chrono::{
    DateTime,
    Utc,
};
use color_eyre::eyre::Error;
use image::RgbaImage;
use nalgebra::{
    Point3,
    Vector3,
};
use palette::WithAlpha;
use serde::{
    Deserialize,
    Serialize,
};
use taffy::prelude::{
    TaffyAuto,
    TaffyZero,
};
use winit::keyboard::KeyCode;

use crate::{
    app::{
        Time,
        WindowConfig,
    },
    build_info::BUILD_INFO,
    ecs::{
        background_tasks::BackgroundTaskConfig,
        plugin::{
            Plugin,
            WorldBuilder,
        },
        schedule,
        transform::{
            GlobalTransform,
            LocalTransform,
        },
    },
    game::{
        block_type::BlockTypes,
        camera_controller::{
            CameraController,
            CameraControllerConfig,
            CameraControllerPlugin,
            CameraControllerState,
        },
        celestial::{
            CelestialFrame,
            GeoCoords,
            world_to_geo,
        },
        file::WorldFile,
        terrain::{
            TerrainGenerator,
            TerrainVoxel,
            WorldConfig,
        },
    },
    input::Keys,
    render::{
        DefaultAtlas,
        RenderConfig,
        RenderSystems,
        atlas::{
            Padding,
            PaddingFill,
            PaddingMode,
        },
        camera::Camera,
        fps_counter::{
            FpsCounter,
            FpsCounterConfig,
        },
        mesh::{
            RenderMeshStatistics,
            RenderWireframes,
        },
        render_target::RenderTarget,
        skybox::{
            Planet,
            Skybox,
            SkyboxPlugin,
        },
        staging::Staging,
        surface::ClearColor,
        text::{
            Text,
            TextColor,
            TextSize,
        },
    },
    ui::{
        Background,
        ShowDebugOutlines,
        Sprites,
        Style,
        View,
    },
    util::{
        format_size,
        image::ImageLoadExt,
        stats_alloc::bytes_allocated,
    },
    voxel::{
        chunk_generator::ChunkGeneratorPlugin,
        chunk_map::{
            ChunkMapPlugin,
            ChunkPosition,
            ChunkStatistics,
        },
        loader::{
            ChunkLoader,
            ChunkLoaderPlugin,
        },
        mesh::{
            ChunkMeshPlugin,
            greedy_quads::GreedyMesher,
        },
    },
    wgpu::WgpuContext,
};

pub const CHUNK_SIZE: usize = 32;

#[derive(Clone, Debug, Default)]
pub struct GamePlugin {
    pub game_config: GameConfig,
    pub init_world: InitWorld,
}

#[derive(Clone, Debug, Resource, Serialize, Deserialize)]
pub struct GameConfig {
    #[serde(default = "default_chunk_distance")]
    pub chunk_load_distance: u32,

    #[serde(default = "default_chunk_distance")]
    pub chunk_render_distance: u32,

    #[serde(default)]
    pub chunk_generator_config: BackgroundTaskConfig,

    #[serde(default)]
    pub camera_controller: CameraControllerConfig,
}

fn default_chunk_distance() -> u32 {
    4
}

impl Default for GameConfig {
    fn default() -> Self {
        Self {
            chunk_load_distance: default_chunk_distance(),
            chunk_render_distance: default_chunk_distance(),
            chunk_generator_config: Default::default(),
            camera_controller: Default::default(),
        }
    }
}

#[derive(Clone, Debug, Resource)]
pub enum InitWorld {
    Load {
        world_file: PathBuf,
    },
    Create {
        world_config: WorldConfig,
        world_file: Option<PathBuf>,
    },
}

impl Default for InitWorld {
    fn default() -> Self {
        Self::Create {
            world_config: Default::default(),
            world_file: None,
        }
    }
}

impl Plugin for GamePlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        match &self.init_world {
            InitWorld::Load { world_file } => {
                let world_file = WorldFile::open(world_file)?;

                builder
                    .insert_resource(world_file.world_config().clone())
                    .insert_resource(world_file);
            }
            InitWorld::Create {
                world_config,
                world_file,
            } => {
                builder.insert_resource(world_config.clone());

                if let Some(world_file) = world_file {
                    let world_file = WorldFile::create(&world_file, world_config.clone())?;
                    builder.insert_resource(world_file);
                }
            }
        }

        builder
            .insert_resource(self.game_config.clone())
            .insert_resource({
                // for debugging
                AstroTime(Utc::now())
            })
            .add_plugin(CameraControllerPlugin)?
            .add_plugin(ChunkMeshPlugin::<
                TerrainVoxel,
                GreedyMesher<TerrainVoxel, CHUNK_SIZE>,
                //NaiveMesher,
                CHUNK_SIZE,
            >::default())?
            .add_plugin(ChunkMapPlugin)?
            .add_plugin(ChunkLoaderPlugin::<CHUNK_SIZE>)?
            .add_plugin(ChunkGeneratorPlugin::<
                TerrainVoxel,
                TerrainGenerator,
                //TestChunkGenerator,
                CHUNK_SIZE,
            >::new(self.game_config.chunk_generator_config))?
            .add_plugin(SkyboxPlugin)?
            .add_systems(
                schedule::Startup,
                (
                    (load_block_types, create_skybox).in_set(RenderSystems::Setup),
                    create_terrain_generator.after(load_block_types),
                    init_player.after(RenderSystems::Setup),
                ),
            )
            .add_systems(schedule::Update, update_sky)
            .add_systems(
                schedule::Render,
                (
                    update_debug_overlay.run_if(
                        resource_changed::<FpsCounter>.and(any_with_component::<DebugOverlay>),
                    ),
                    handle_keys,
                ),
            );

        Ok(())
    }
}

fn load_block_types(
    mut atlas: ResMut<DefaultAtlas>,
    wgpu: Res<WgpuContext>,
    mut staging: ResMut<Staging>,
    mut commands: Commands,
) {
    let block_types = BlockTypes::load("assets/blocks.toml", |image| {
        Ok(atlas.insert_image(
            image,
            Some(PaddingMode {
                padding: Padding::uniform(1),
                fill: PaddingFill::REPEAT,
            }),
            &wgpu.device,
            &mut *staging,
        )?)
    })
    .unwrap();
    commands.insert_resource(block_types);
}

fn create_skybox(
    wgpu: Res<WgpuContext>,
    mut atlas: ResMut<DefaultAtlas>,
    mut staging: ResMut<Staging>,
    mut commands: Commands,
) {
    let skybox = Skybox::load(&wgpu, "assets/skybox").unwrap();

    let mut make_planet = |id, path, size| {
        // with a realistic planet size the sun and moon would only be a few pixels in
        // diameter. e.g. with a fov of 60°, an angular diameter of 0.5° and a
        // screen size of 1024 pixels, the planet would only be 8.5 pixels.
        //
        // thus we just make it larger
        let size = size * 4.0;

        let image = RgbaImage::from_path(path).unwrap();

        let atlas_handle = atlas
            .insert_image(&image, None, &wgpu.device, &mut staging)
            .unwrap();

        tracing::debug!(?path, ?atlas_handle, "loaded texture");

        (
            Name::new(format!("{id:?}")),
            Planet {
                texture: atlas_handle,
                size,
            },
            GlobalTransform::identity(),
            id,
        )
    };

    commands
        .spawn((skybox, GlobalTransform::identity()))
        .with_children(|spawner| {
            spawner.spawn(make_planet(
                PlanetId::Sun,
                "assets/skybox/sun.png",
                // average angular size
                0.536f32.to_radians(),
            ));
            spawner.spawn(make_planet(
                PlanetId::Moon,
                "assets/skybox/moon.png",
                // average angular size
                0.528f32.to_radians(),
            ));
        });
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Component)]
enum PlanetId {
    Sun,
    Moon,
}

fn create_terrain_generator(
    block_types: Res<BlockTypes>,
    world_config: Res<WorldConfig>,
    mut commands: Commands,
) {
    commands.insert_resource(TerrainGenerator::new(&world_config, &block_types));
    //commands.insert_resource(TestChunkGenerator::new(&block_types));
}

fn init_player(
    config: Res<GameConfig>,
    render_config: Res<RenderConfig>,
    sprites: Res<Sprites>,
    mut fps_counter_config: ResMut<FpsCounterConfig>,
    mut commands: Commands,
) {
    tracing::debug!("initializing world");

    let chunk_side_length = CHUNK_SIZE as f32;
    let chunk_center = Point3::from(Vector3::repeat(0.5 * chunk_side_length));

    // spawn window
    let window = commands
        .spawn((
            Name::new("main_window"),
            WindowConfig {
                title: "SandVox".to_owned(),
            },
        ))
        .id();

    // spawn camera
    commands.spawn((
        Name::new("main_camera"),
        RenderTarget(window),
        ClearColor(palette::named::LIGHTSKYBLUE.into_format().with_alpha(1.0)),
        Camera {
            aspect_ratio: 1.0,
            fovy: render_config.fov.to_radians(),
            z_near: 0.1,
            z_far: config.chunk_render_distance as f32 * CHUNK_SIZE as f32,
        },
        LocalTransform::from(chunk_center + chunk_side_length * Vector3::y()),
        CameraController {
            state: CameraControllerState {
                yaw: 0.0,
                pitch: 0.0,
            },
            config: config.camera_controller.clone(),
        },
        ChunkLoader {
            radius: Vector3::repeat(config.chunk_load_distance),
        },
        Player,
    ));

    {
        // create UI
        fps_counter_config.measurement_inverval = Duration::from_millis(100);
        let pixel_size = 2.0;
        let text_style = (
            TextSize {
                scaling: pixel_size,
            },
            TextColor {
                color: palette::named::WHITESMOKE.into_format().with_alpha(1.0),
            },
        );

        commands
            .spawn((Name::new("ui"), View::default(), RenderTarget(window)))
            .with_children(|ui| {
                // create debug panel
                ui.spawn({
                    let sprite = &sprites["panel"];
                    let background = Background {
                        sprite: sprite.clone(),
                        pixel_size,
                    };

                    let mut style = Style::default();
                    style.display = taffy::style::Display::Flex;
                    style.flex_direction = taffy::style::FlexDirection::Column;
                    style.margin = taffy::Rect {
                        left: taffy::LengthPercentageAuto::ZERO,
                        right: taffy::LengthPercentageAuto::AUTO,
                        top: taffy::LengthPercentageAuto::ZERO,
                        bottom: taffy::LengthPercentageAuto::AUTO,
                    };
                    if let Some(padding) = sprite.padding(pixel_size) {
                        style.padding = padding;
                    }

                    (style, background, Name::new("debug_panel"))
                })
                .with_children(|panel| {
                    panel.spawn((
                        Name::new("build_tag"),
                        Text::from(format_build_tag()),
                        text_style,
                        Style::default(),
                    ));
                    panel.spawn((
                        Name::new("debug_info"),
                        Text::default(),
                        text_style,
                        Style::default(),
                        DebugOverlay,
                    ));
                });

                // create crosshair
                ui.spawn({
                    let sprite = &sprites["crosshair"];
                    let background = Background {
                        sprite: sprite.clone(),
                        pixel_size,
                    };

                    let mut style = Style::default();
                    style.display = taffy::style::Display::Block;
                    style.position = taffy::Position::Absolute;
                    style.margin = taffy::Rect::auto();
                    style.size = taffy::Size::from_lengths(
                        sprite.size.x as f32 * pixel_size,
                        sprite.size.y as f32 * pixel_size,
                    );

                    (Name::new("crosshair"), style, background)
                });
            });
    }
}

fn format_build_tag() -> String {
    let mut s = String::with_capacity(64);

    write!(&mut s, "SANDVOX: ").unwrap();

    match BUILD_INFO.profile {
        "release" => {
            write!(&mut s, "{}", BUILD_INFO.version).unwrap();
        }
        _ => {
            write!(&mut s, "DEV").unwrap();
            if let Some(branch) = BUILD_INFO.git_branch {
                write!(&mut s, "/{}", &branch).unwrap();
            }
            if let Some(commit) = BUILD_INFO.git_commit {
                write!(&mut s, "/{}", &commit[..7]).unwrap();
            }
        }
    }

    s
}

#[derive(Clone, Copy, Debug, Default, Component)]
struct DebugOverlay;

fn update_debug_overlay(
    fps_counter: Res<FpsCounter>,
    wgpu: Res<WgpuContext>,
    time: Res<Time>,
    render_mesh: Res<RenderMeshStatistics>,
    mut debug_overlay: Single<&mut Text, With<DebugOverlay>>,
    player: Option<Single<&GlobalTransform, With<Player>>>,
    astro_time: Res<AstroTime>,
    chunks: Query<(), With<ChunkPosition>>,
    chunk_statistics: Res<ChunkStatistics>,
) {
    debug_overlay.text.clear();

    writeln!(
        &mut debug_overlay.text,
        "TIME: N={}, T={:.1}s, DT={:.1}ms, W={}",
        time.tick_count,
        time.tick_start_seconds(),
        time.delta_seconds() * 1000.0,
        astro_time.0.format("%Y-%m-%d %H:%M"),
    )
    .unwrap();

    writeln!(&mut debug_overlay.text, "FPS: {:.1}", fps_counter.fps).unwrap();

    write!(
        &mut debug_overlay.text,
        "MEM: CPU={}",
        format_size(bytes_allocated())
    )
    .unwrap();

    if let Some(allocator_report) = wgpu.device.generate_allocator_report() {
        writeln!(
            &mut debug_overlay.text,
            ", GPU={}",
            format_size(allocator_report.total_allocated_bytes)
        )
        .unwrap();
    }
    else {
        writeln!(&mut debug_overlay.text, "").unwrap();
    }

    let staging_info = wgpu.staging_pool.info();
    writeln!(
        &mut debug_overlay.text,
        "STAGING: INFLIGHT={}, FREE={}, TOTAL={}/{}",
        staging_info.in_flight_count,
        staging_info.free_count,
        staging_info.total_allocation_count,
        format_size(staging_info.total_allocation_bytes)
    )
    .unwrap();

    writeln!(
        &mut debug_overlay.text,
        "MESH: DRAW={}, VERT={}, CULL={}",
        render_mesh.num_rendered, render_mesh.num_vertices, render_mesh.num_culled,
    )
    .unwrap();

    writeln!(
        &mut debug_overlay.text,
        "CHUNK: T={}, L={}/{}, M={}/{}",
        chunks.count(),
        chunk_statistics.num_chunks_loaded,
        format_size(chunk_statistics.bytes_chunks_loaded),
        chunk_statistics.num_chunks_meshed,
        format_size(chunk_statistics.bytes_chunks_meshed),
    )
    .unwrap();

    if let Some(transform) = player {
        let position = transform.position();
        let look_dir = transform.isometry * Vector3::z();
        writeln!(
            &mut debug_overlay.text,
            "POS: {:.1}, {:.1}, {:.1}; LOOK: {:.1}, {:.1}, {:.1}",
            position.x, position.y, position.z, look_dir.x, look_dir.y, look_dir.z,
        )
        .unwrap();
    }
}

fn handle_keys(
    keys: Populated<&Keys, Changed<Keys>>,
    render_wireframes: Option<Res<RenderWireframes>>,
    show_ui_layout: Option<Res<ShowDebugOutlines>>,
    mut commands: Commands,
) {
    for keys in keys {
        if keys.just_pressed.contains(&KeyCode::F6) {
            if render_wireframes.is_none() {
                commands.insert_resource(RenderWireframes);
            }
            else {
                commands.remove_resource::<RenderWireframes>();
            }
        }

        if keys.just_pressed.contains(&KeyCode::F7) {
            if show_ui_layout.is_none() {
                commands.insert_resource(ShowDebugOutlines);
            }
            else {
                commands.remove_resource::<ShowDebugOutlines>();
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Component)]
struct Player;

fn update_sky(
    mut params: ParamSet<(
        Single<&GlobalTransform, With<Player>>,
        Single<&mut GlobalTransform, With<Skybox>>,
        Query<(&mut GlobalTransform, &PlanetId)>,
    )>,
    time: Res<Time>,
    mut astro_time: ResMut<AstroTime>,
) {
    const WORLD_ORIGIN: GeoCoords<f64> = GeoCoords {
        // what's here?
        latitude: 51.283889f64.to_radians(),
        longitude: 11.52f64.to_radians(),
    };

    const DAY_LENGTH: f32 = 600.0;
    const TIME_WARP: f32 = 24.0 * 60.0 * 60.0 / DAY_LENGTH;

    let observer = world_to_geo(params.p0().position(), WORLD_ORIGIN);
    let time = time.app_start_utc + Duration::from_secs_f32(TIME_WARP * time.tick_start_seconds());
    //let time = Utc::now();
    let frame = CelestialFrame::new(observer, time);

    params.p1().isometry.rotation = frame.sky();

    for (mut planet_transform, planet_id) in params.p2() {
        planet_transform.isometry.rotation = match planet_id {
            PlanetId::Sun => frame.sun(),
            PlanetId::Moon => frame.moon(),
        };
    }

    astro_time.0 = time;
}

#[derive(Debug, Resource)]
struct AstroTime(DateTime<Utc>);
