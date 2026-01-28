// use by crate::util::stats_alloc
#![feature(allocator_api)]

pub mod app;
pub mod build_info;
pub mod collide;
pub mod config;
pub mod ecs;
pub mod game;
pub mod input;
pub mod profiler;
pub mod render;
pub mod sound;
pub mod ui;
pub mod util;
pub mod voxel;
pub mod wgpu;

use clap::{
    Parser,
    Subcommand,
};
use color_eyre::eyre::Error;

use crate::{
    app::App,
    wgpu::WgpuContextBuilder,
};

#[derive(Debug, Parser)]
pub struct Args {
    #[clap(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    Main(crate::app::Args),
    WgpuInfo,
}

impl Default for Command {
    fn default() -> Self {
        Self::Main(Default::default())
    }
}

fn main() -> Result<(), Error> {
    let _ = dotenvy::dotenv();
    color_eyre::install()?;
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    match args.command.unwrap_or_default() {
        Command::Main(args) => {
            let app = App::new(args)?;
            app.run()?;
        }
        Command::WgpuInfo => {
            wgpu_info()?;
        }
    }

    Ok(())
}

fn wgpu_info() -> Result<(), Error> {
    let builder = WgpuContextBuilder::new(Default::default())?;

    println!("supported features:");
    for (feature, _) in builder.supported_features.iter_names() {
        println!("  {feature}");
    }

    println!("supported limits: {:#?}", builder.supported_limits);

    let context = builder.build(None)?;
    println!("adapter info: {:#?}", context.info.adapter);

    Ok(())
}
