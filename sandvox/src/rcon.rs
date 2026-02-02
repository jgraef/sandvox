use std::net::SocketAddr;

use bevy_ecs::{
    entity::Entity,
    query::With,
    resource::Resource,
    system::{
        In,
        InMut,
        IntoSystem,
        Query,
        Single,
    },
    world::World,
};
use color_eyre::eyre::{
    Error,
    eyre,
};
use futures_lite::StreamExt;
use nalgebra::Vector3;
use sandvox_rcon::{
    Command,
    TeleportCommand,
};
use serde::{
    Deserialize,
    Serialize,
};
use tokio::{
    net::{
        TcpListener,
        TcpStream,
    },
    sync::{
        mpsc,
        oneshot,
    },
    task::JoinHandle,
};
use tokio_util::codec::{
    Framed,
    LinesCodec,
};
use tracing::{
    Instrument,
    Span,
};

use crate::{
    ecs::{
        plugin::{
            Plugin,
            WorldBuilder,
        },
        schedule,
        transform::LocalTransform,
    },
    game::Player,
    util::tokio::TokioRuntime,
};

#[derive(Clone, Debug)]
pub struct RconPlugin {
    pub config: RconConfig,
}

impl Plugin for RconPlugin {
    fn setup(&self, builder: &mut WorldBuilder) -> Result<(), Error> {
        let rt = builder.world.resource::<TokioRuntime>();
        let (shutdown_sender, shutdown_receiver) = oneshot::channel::<()>();
        let (queue_sender, queue_receiver) = mpsc::channel(32);

        let join_handle = rt.spawn({
            let address = self.config.address.clone();

            async move {
                run_server(address, shutdown_receiver, queue_sender)
                    .await
                    .inspect_err(|error| {
                        tracing::error!(%error);
                    })
            }
        });

        builder
            .insert_resource(RconServer {
                _shutdown_sender: shutdown_sender,
                _join_handle: join_handle,
            })
            .add_systems(schedule::Update, handle_commands.with_input(queue_receiver));

        Ok(())
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RconConfig {
    pub address: String,
}

#[derive(Debug, Resource)]
pub struct RconServer {
    /// This will shutdown the server task when dropped.
    _shutdown_sender: oneshot::Sender<()>,

    /// do we need this?
    _join_handle: JoinHandle<Result<(), Error>>,
}

fn handle_commands(
    InMut(queue_receiver): InMut<mpsc::Receiver<(Span, Command)>>,
    world: &mut World,
) {
    loop {
        match queue_receiver.try_recv() {
            Ok((span, command)) => {
                let _guard = span.enter();

                let result = match command {
                    Command::TeleportCommand(teleport_command) => {
                        teleport_command.handle_command(world)
                    }
                };

                if let Err(error) = result {
                    tracing::error!(%error);
                }
            }
            Err(mpsc::error::TryRecvError::Disconnected) => {
                world.remove_resource::<RconServer>();
            }
            Err(mpsc::error::TryRecvError::Empty) => break,
        }
    }
}

async fn run_server(
    address: String,
    mut shutdown: oneshot::Receiver<()>,
    queue_sender: mpsc::Sender<(Span, Command)>,
) -> Result<(), Error> {
    let listener = TcpListener::bind(&address).await?;
    tracing::info!("RCON server listening on `{address}`");

    loop {
        tokio::select! {
            _ = &mut shutdown => {
                break;
            }
            result = listener.accept() => {
                let (stream, address) = result?;
                let span = tracing::info_span!("rcon client", ?address);
                let queue_sender = queue_sender.clone();
                tokio::spawn(async move {
                    if let Err(error) = handle_connection(stream, address, queue_sender).await {
                        tracing::error!(%error);
                    }
                }.instrument(span));
            }
        }
    }

    tracing::debug!("RCON server shutting down");

    Ok(())
}

async fn handle_connection(
    stream: TcpStream,
    _address: SocketAddr,
    queue: mpsc::Sender<(Span, Command)>,
) -> Result<(), Error> {
    let codec = LinesCodec::new();
    let mut framed = Framed::new(stream, codec);

    tracing::info!("rcon client connected");

    while let Some(line) = framed.try_next().await? {
        let command: Command = serde_json::from_str(&line)?;
        tracing::debug!(?command);

        queue.send((Span::current(), command)).await?;
    }

    tracing::info!("rcon client disconnected");

    Ok(())
}

trait HandleCommand {
    fn handle_command(self, world: &mut World) -> Result<(), Error>;
}

impl HandleCommand for TeleportCommand {
    fn handle_command(self, world: &mut World) -> Result<(), Error> {
        world
            .run_system_cached_with(
                |In(command): In<TeleportCommand>,
                 player: Option<Single<Entity, With<Player>>>,
                 mut entities: Query<&mut LocalTransform>| {
                    let entity = command
                        .entity
                        .map(|entity| Entity::from_bits(entity.0))
                        .or_else(|| player.as_deref().copied())
                        .ok_or_else(|| eyre!("No entity specified and no player found"))?;

                    let mut transform = entities.get_mut(entity)?;
                    transform.isometry.translation.vector = Vector3::new(
                        command.destination.x,
                        command.destination.y,
                        command.destination.z,
                    );

                    //todo!();
                    Ok::<(), Error>(())
                },
                self,
            )
            .unwrap()
    }
}
