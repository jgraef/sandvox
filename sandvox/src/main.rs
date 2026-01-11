pub mod app;
pub mod config;
pub mod ecs;
pub mod input;
pub mod render;
pub mod util;
pub mod voxel;
pub mod wgpu;
pub mod world;

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

#[derive(Debug, Subcommand, Default)]
enum Command {
    #[default]
    Main,
    WgpuInfo,
}

fn main() -> Result<(), Error> {
    let _ = dotenvy::dotenv();
    color_eyre::install()?;
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    match args.command.unwrap_or_default() {
        Command::Main => {
            let app = App::new()?;
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

    let context = builder.build()?;
    println!("adapter info: {:#?}", context.info.adapter);

    Ok(())
}
