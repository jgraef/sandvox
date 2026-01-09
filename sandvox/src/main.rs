pub mod app;
pub mod ecs;
pub mod render;
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
    wgpu::WgpuContext,
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
    let wgpu = WgpuContext::new(&Default::default())?;
    println!("{:#?}", wgpu.info);
    Ok(())
}
