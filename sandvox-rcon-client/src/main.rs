use std::fmt::Debug;

use clap::Parser;
use color_eyre::eyre::Error;
use sandvox_rcon::Command;
use sandvox_rcon_client::RconClient;

#[tokio::main]
async fn main() -> Result<(), Error> {
    let _ = dotenvy::dotenv();
    color_eyre::install()?;
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    let mut client = RconClient::connect(&args.address).await?;
    client.send(&args.command).await?;

    Ok(())
}

#[derive(Debug, Parser)]
struct Args {
    #[clap(short, long, default_value = "localhost:25576")]
    address: String,

    #[clap(subcommand)]
    command: Command,
}
