pub mod skybox;
pub mod tres;

use std::path::PathBuf;

use clap::{
    Parser,
    Subcommand,
};
use color_eyre::eyre::Error;

#[derive(Clone, Debug, Parser)]
struct Args {
    #[clap(subcommand)]
    command: Command,
}

#[derive(Clone, Debug, Subcommand)]
enum Command {
    ParseTres {
        #[clap(short, long)]
        recursive: bool,

        path: PathBuf,
    },
    MakeSkybox {
        #[clap(short, long)]
        output: PathBuf,

        #[clap(short, long)]
        layers: Vec<PathBuf>,

        #[clap(short, long, default_value = "1024")]
        size: u32,

        stars: PathBuf,
    },
}

fn main() -> Result<(), Error> {
    let _ = dotenvy::dotenv();
    color_eyre::install()?;
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    match args.command {
        Command::ParseTres { recursive, path } => {
            tres::parse_tres(path, recursive)?;
        }
        Command::MakeSkybox {
            output,
            layers,
            size,
            stars,
        } => {
            skybox::make_skybox(stars, layers, size, output)?;
        }
    }

    Ok(())
}
