use std::fmt::Debug;

use color_eyre::eyre::Error;
use futures_util::SinkExt;
pub use sandvox_rcon::*;
use tokio::net::{
    TcpStream,
    ToSocketAddrs,
};
use tokio_util::codec::{
    Framed,
    LinesCodec,
};

#[derive(Debug)]
pub struct RconClient {
    framed: Framed<TcpStream, LinesCodec>,
}

impl RconClient {
    pub async fn connect<A>(address: A) -> Result<Self, Error>
    where
        A: ToSocketAddrs + Debug,
    {
        let stream = TcpStream::connect(&address).await?;
        tracing::info!(?address, "connected");

        let codec = LinesCodec::new();
        let framed = Framed::new(stream, codec);

        Ok(Self { framed })
    }

    pub async fn send(&mut self, command: &Command) -> Result<(), Error> {
        let json = serde_json::to_string(command)?;
        self.framed.send(&json).await?;
        Ok(())
    }
}
