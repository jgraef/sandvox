use std::path::Path;

use bevy_ecs::resource::Resource;
use chrono::{
    DateTime,
    Local,
};
use color_eyre::eyre::{
    Error,
    OptionExt,
};
use redb::{
    Database,
    ReadableDatabase,
    TableDefinition,
};
use serde::{
    Deserialize,
    Serialize,
};

use crate::world::terrain::WorldSeed;

#[derive(Debug, Resource)]
pub struct WorldFile {
    _database: Database,
    metadata: Metadata,
}

impl WorldFile {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, Error> {
        let database = Database::open(path)?;

        let read_transaction = database.begin_read()?;
        let table = read_transaction.open_table(METADATA)?;
        let metadata: Metadata =
            serde_cbor::from_slice(&table.get(())?.ok_or_eyre("no metadata")?.value())?;

        Ok(Self {
            _database: database,
            metadata,
        })
    }

    pub fn create(path: impl AsRef<Path>, seed: WorldSeed) -> Result<Self, Error> {
        let database = Database::create(path)?;

        let time = Local::now();
        let metadata = Metadata {
            world_seed: seed,
            time_created: time,
            time_last_written: time,
        };

        let write_transaction = database.begin_write()?;
        {
            let mut table = write_transaction.open_table(METADATA)?;
            table.insert((), serde_cbor::to_vec(&metadata)?)?;
        }
        write_transaction.commit()?;

        Ok(Self {
            _database: database,
            metadata,
        })
    }

    pub fn world_seed(&self) -> WorldSeed {
        self.metadata.world_seed
    }
}

const METADATA: TableDefinition<(), Vec<u8>> = TableDefinition::new("metadata");

#[derive(Debug, Serialize, Deserialize)]
struct Metadata {
    world_seed: WorldSeed,
    time_created: DateTime<Local>,
    time_last_written: DateTime<Local>,
}
