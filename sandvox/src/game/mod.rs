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
        Populated,
        Res,
        ResMut,
        Single,
    },
};
use color_eyre::eyre::Error;
use nalgebra::{
    Point3,
    Vector3,
};
use palette::WithAlpha;
use serde::{
    Deserialize,
    Serialize,
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
            GeoCoords,
            sky_orientation,
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
        frame::DefaultAtlas,
        mesh::{
            RenderMeshStatistics,
            RenderWireframes,
        },
        skybox::{
            Skybox,
            SkyboxPlugin,
        },
        staging::Staging,
        surface::{
            ClearColor,
            RenderTarget,
        },
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
    },
    util::{
        format_size,
        stats_alloc::bytes_allocated,
    },
    voxel::{
        chunk_generator::ChunkGeneratorPlugin,
        chunk_map::ChunkMapPlugin,
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
                    load_block_types.in_set(RenderSystems::Setup),
                    create_terrain_generator.after(load_block_types),
                    init_player.after(RenderSystems::Setup),
                ),
            )
            .add_systems(schedule::Update, rotate_skybox)
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
            ClearColor(palette::named::LIGHTSKYBLUE.into_format().with_alpha(1.0)),
        ))
        .id();

    // spawn camera
    commands.spawn((
        Name::new("main_camera"),
        RenderTarget(window),
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

    commands.spawn((
        Skybox {
            path: "assets/skybox/test_skybox".into(),
        },
        LocalTransform::identity(),
    ));

    // create cursor
    // todo

    // create debug ui
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
        .spawn({
            let sprite = &sprites["panel"];
            let background = Background {
                sprite: sprite.clone(),
                pixel_size,
            };

            let mut style = Style::default();
            style.display = taffy::style::Display::Flex;
            style.flex_direction = taffy::style::FlexDirection::Column;
            if let Some(padding) = sprite.padding(pixel_size) {
                style.padding = padding;
            }

            (
                style,
                background,
                RenderTarget(window),
                Name::new("debug_panel"),
            )
        })
        .with_children(|spawner| {
            spawner.spawn((Text::from(format_build_tag()), text_style, Style::default()));
            spawner.spawn((Text::default(), text_style, Style::default(), DebugOverlay));
        });

    // create crosshair
    commands
        .spawn({
            let mut style = Style::default();
            style.display = taffy::style::Display::Flex;
            style.size = taffy::Size::percent(1.0);

            (style, RenderTarget(window), Name::new("crosshair"))
        })
        .with_child({
            let sprite = &sprites["crosshair"];
            let background = Background {
                sprite: sprite.clone(),
                pixel_size,
            };

            let mut style = Style::default();
            style.display = taffy::style::Display::Flex;
            style.margin = taffy::Rect::auto();
            style.size = taffy::Size::from_lengths(
                sprite.size.x as f32 * pixel_size,
                sprite.size.y as f32 * pixel_size,
            );
            style.align_self = Some(taffy::AlignSelf::Center);

            (style, background)
        });
}

fn format_build_tag() -> String {
    match BUILD_INFO.profile {
        "release" => {
            format!("SANDVOX: {}", BUILD_INFO.version)
        }
        _ => {
            if let Some(commit) = BUILD_INFO.git_commit {
                format!("SANDVOX: DEV/{}", &commit[..7])
            }
            else {
                format!("SANDVOX: DEV/UNKNOWN")
            }
        }
    }
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
) {
    debug_overlay.text.clear();

    writeln!(
        &mut debug_overlay.text,
        "TIME: N={}, T={:.1}s, DT={:.1}ms",
        time.tick_count,
        time.tick_start_seconds(),
        time.delta_seconds() * 1000.0
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

    if let Some(transform) = player {
        let position = transform.position();
        let look_dir = transform.isometry() * Vector3::z();
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

fn rotate_skybox(
    mut skybox: Single<&mut LocalTransform, With<Skybox>>,
    player: Single<&GlobalTransform, With<Player>>,
    time: Res<Time>,
) {
    const WORLD_ORIGIN: GeoCoords<f64> = GeoCoords {
        // what's here?
        latitude: 51.283889f64.to_radians(),
        longitude: 11.52f64.to_radians(),
    };

    const DAY_LENGTH: f32 = 600.0;
    const TIME_WARP: f32 = 24.0 * 60.0 * 60.0 / DAY_LENGTH;

    let observer = player.position();
    let observer = world_to_geo(observer, WORLD_ORIGIN);

    let time = time.app_start_utc + Duration::from_secs_f32(TIME_WARP * time.tick_start_seconds());

    skybox.isometry.rotation = sky_orientation(observer, time);
}
