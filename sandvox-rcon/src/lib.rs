use serde::{
    Deserialize,
    Serialize,
};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, derive_more::FromStr)]
#[serde(transparent)]
pub struct Entity(pub u64);

#[derive(Clone, Copy, Debug, Serialize, Deserialize, clap::Args)]
pub struct Vec3 {
    pub x: f32,
    pub y: f32,
    pub z: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, clap::Parser)]
pub struct TeleportCommand {
    #[clap(short, long)]
    pub entity: Option<Entity>,

    #[clap(flatten)]
    pub destination: Vec3,
}

#[derive(Clone, Debug, Serialize, Deserialize, clap::Subcommand)]
#[serde(rename_all = "kebab-case")]
pub enum Command {
    TeleportCommand(TeleportCommand),
}
