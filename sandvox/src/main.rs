pub mod app;
pub mod ecs;
pub mod render;
pub mod util;
pub mod wgpu;

use clap::Parser;
use color_eyre::eyre::Error;

use crate::app::{
    App,
    Args,
};

fn main() -> Result<(), Error> {
    let _ = dotenvy::dotenv();
    color_eyre::install()?;
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    let app = App::new(args)?;
    app.run()?;

    Ok(())
}
