//! The PostgreSQL migration session protocol, shared by the sync and async
//! drivers. Mode-free parts are plain code here; the IO steps are written once
//! in [`__pg_migrations!`](crate::__pg_migrations), which each driver
//! instantiates for its mode on its own `PgExecutor`, since `postgres::Client`
//! and `tokio_postgres::Client` share method names. Not public API.

mod control;
mod session;
pub mod sql;

use std::time::Duration;

pub use rupa_core;
pub use rupa_migrate;
use rupa_migrate::{Checksum, Engine, Error, HistoryEntry, StateError, Target};
pub use rupa_sql;
use tokio_postgres::Row;
pub use tokio_postgres::SimpleQueryMessage;
pub use tokio_postgres::error::SqlState;

use crate::PgError;

/// PostgreSQL's largest `lock_timeout`; longer waits are clamped to it.
const MAX_LOCK_TIMEOUT_MS: u128 = i32::MAX as u128;

/// The advisory lock a connection holds and the namespace it owns.
#[derive(Debug)]
pub struct Owner {
    /// The advisory lock key derived from the canonical target.
    pub key: i64,
    /// The schema owning the migration metadata.
    pub namespace: String,
}

/// Per-connection ownership, attempt and transaction state.
#[derive(Debug)]
pub struct State {
    /// The held lock and owned namespace while a run owns the target.
    pub owner: Option<Owner>,
    /// The journaled attempt in progress.
    pub attempt: Option<i64>,
    /// The transaction id of the open artifact transaction.
    pub transaction: Option<i64>,
    /// An uncertain outcome: the connection must be discarded.
    pub poisoned: bool,
    /// How long to wait for another runner's lock.
    pub lock_timeout: Duration,
}

impl Default for State {
    fn default() -> Self {
        Self {
            owner: None,
            attempt: None,
            transaction: None,
            poisoned: false,
            lock_timeout: Duration::from_secs(5),
        }
    }
}

impl State {
    /// Whether user statements may run: owned, unpoisoned, inside an attempt's transaction.
    pub fn user_state(&self) -> Result<(), StateError> {
        if self.owner.is_none() {
            Err(StateError::NotOwner)
        } else if self.poisoned {
            Err(StateError::Poisoned)
        } else if self.attempt.is_none() {
            Err(StateError::NoAttempt)
        } else if self.transaction.is_none() {
            Err(StateError::NoTransaction)
        } else {
            Ok(())
        }
    }

    /// The `lock_timeout` in milliseconds, at least one, clamped to PostgreSQL's maximum.
    pub fn lock_millis(&self) -> u128 {
        self.lock_timeout.as_millis().clamp(1, MAX_LOCK_TIMEOUT_MS)
    }

    /// The owned namespace, unquoted.
    pub fn owned_namespace(&self) -> Result<String, Error<PgError>> {
        self.owner
            .as_ref()
            .map(|owner| owner.namespace.clone())
            .ok_or_else(|| Error::History("migration connection does not own the target".into()))
    }

    /// The qualified name of a reserved metadata table in the owned namespace.
    pub fn reserved(&self, table: &str) -> Result<String, Error<PgError>> {
        Ok(format!("{}.{table}", quote(&self.owned_namespace()?)))
    }
}

/// What a clean migration connection reports about itself.
#[derive(Debug)]
pub struct Connection {
    /// `current_database()`.
    pub database: String,
    /// `current_schema()`, the owned namespace.
    pub namespace: String,
}

impl Connection {
    /// Reads the [`sql::DESCRIBE`] row, rejecting connections inside an
    /// application transaction or with settings the script grammar does not assume.
    pub fn describe(messages: Vec<SimpleQueryMessage>) -> Result<Self, Error<PgError>> {
        let mut values = Vec::new();

        for message in messages {
            if let SimpleQueryMessage::Row(row) = message {
                values = (0..4).map(|i| row.get(i).map(str::to_owned)).collect();
            }
        }

        let [autocommit, standard, Some(database), namespace] = values.as_slice() else {
            return Err(Error::History(
                "connection did not describe its database".into(),
            ));
        };

        if autocommit.as_deref() != Some("t") {
            return Err(Error::History(
                "connection is already inside an application transaction".into(),
            ));
        }

        if standard.as_deref() != Some("on") {
            return Err(Error::History(
                "migration connections require standard_conforming_strings = on".into(),
            ));
        }

        let Some(namespace) = namespace else {
            return Err(Error::History(
                "migration connections require a current schema on the search_path".into(),
            ));
        };

        Ok(Self {
            database: database.clone(),
            namespace: namespace.clone(),
        })
    }

    /// The advisory lock key of this target.
    pub fn key(&self) -> i64 {
        Target {
            engine: Engine::Postgres,
            database: &self.database,
            namespace: &self.namespace,
        }
        .advisory_key()
    }
}

/// A bigint advisory key as `pg_locks` reports it: high and low 32 bits.
pub fn lock_parts(key: i64) -> (i64, i64) {
    (i64::from((key >> 32) as u32), i64::from(key as u32))
}

/// Quotes an identifier for interpolation into framework-owned SQL.
pub fn quote(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

/// A driver failure as a migration error.
pub fn db(error: tokio_postgres::Error) -> Error<PgError> {
    Error::Database(PgError::Db(error))
}

/// Decodes [`sql::history`] rows.
pub fn decode_history(rows: Vec<Row>) -> Result<Vec<HistoryEntry>, Error<PgError>> {
    rows.into_iter()
        .map(|row| {
            let installation_rank = u64::try_from(row.get::<_, i64>(0))
                .ok()
                .filter(|rank| *rank > 0)
                .ok_or_else(|| Error::History("invalid installation rank".into()))?;
            let bytes = row
                .get::<_, Vec<u8>>(2)
                .try_into()
                .map_err(|_| Error::History("invalid stored checksum length".into()))?;

            Ok(HistoryEntry {
                installation_rank,
                version: row.get(1),
                checksum: Checksum::from_bytes(bytes),
            })
        })
        .collect()
}

/// Implements the migration session on the caller's `PgExecutor` for one
/// mode. The executor must have `client`, `dialect`, `statements` and a
/// `migration: State` field, and inherent `run`.
#[doc(hidden)]
#[macro_export]
macro_rules! __pg_migrations {
    (@uses) => {
        use $crate::PgError;
        use $crate::migrations::rupa_migrate::{
            CHECKSUM_FORMAT, CatalogEntry, Error, HistoryEntry, StateError, VersionScheme,
        };
        use $crate::migrations::{Connection, Owner, SqlState, db, decode_history, lock_parts, quote, sql};
    };
    (sync) => {
        $crate::__pg_migrations!(@uses);
        $crate::__pg_migration_control!(
            [] [] $crate::migrations::rupa_core::tx::TxConnection
        );
        $crate::__pg_migration_session!(
            [] [] $crate::migrations::rupa_migrate::sync::Session
        );
    };
    (async) => {
        $crate::__pg_migrations!(@uses);
        $crate::__pg_migration_control!(
            [async] [.await] $crate::migrations::rupa_core::tx::AsyncTxConnection
        );
        $crate::__pg_migration_session!(
            [async] [.await] $crate::migrations::rupa_migrate::asynchronous::Session
        );
    };
}
