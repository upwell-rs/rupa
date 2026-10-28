//! The PostgreSQL migration session protocol: a session advisory lock keyed
//! by [`Target::advisory_key`], the journal table in the current schema, and
//! transactional or autocommit bodies.

mod coverage;
mod inspect;
mod recovery;
pub mod sql;
mod steps;

use std::time::Duration;

use rupa_core::{DialectId, Value};

use crate::{Engine, Error, Policy, StateError, Target};

/// The policies PostgreSQL sessions honour.
pub const POLICIES: &[Policy] = Engine::Postgres.policies();

/// `lock_not_available`, raised when `lock_timeout` expires.
const LOCK_NOT_AVAILABLE: &str = "55P03";

/// PostgreSQL's largest `lock_timeout`; longer waits are clamped to it.
const MAX_LOCK_TIMEOUT_MS: u128 = i32::MAX as u128;

/// The advisory lock a connection holds and the namespace it owns.
#[derive(Debug)]
pub struct Owner {
    /// The advisory lock key derived from the canonical target.
    pub key: i64,
    /// The schema owning the migration journal.
    pub namespace: String,
}

/// How the body of the journaled attempt executes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Body {
    /// Inside the artifact transaction with this transaction id.
    Transaction(i64),
    /// In autocommit, one statement per request, for nontransactional work;
    /// `dispatched` once a statement was sent, so effects may persist.
    Autocommit {
        /// Whether any statement of the body was sent.
        dispatched: bool,
    },
}

/// Per-connection ownership, attempt and body state.
#[derive(Debug)]
pub struct State {
    /// The held lock and owned namespace while a run owns the target.
    pub owner: Option<Owner>,
    /// The journaled attempt in progress.
    pub attempt: Option<i64>,
    /// The running attempt's body, once begun.
    pub body: Option<Body>,
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
            body: None,
            poisoned: false,
            lock_timeout: Duration::from_secs(5),
        }
    }
}

impl State {
    /// Whether a run left state on the connection: owned, mid-attempt,
    /// mid-body or poisoned. Pools must discard such connections.
    pub fn is_active(&self) -> bool {
        self.owner.is_some() || self.attempt.is_some() || self.body.is_some() || self.poisoned
    }

    /// Refuses a statement sent to the connection around a run rather than
    /// through its migration context: while a run owns the connection, or
    /// once a run poisoned it.
    pub fn outside_run(&self) -> Result<(), StateError> {
        if self.poisoned {
            Err(StateError::Poisoned)
        } else if self.owner.is_some() {
            Err(StateError::InRun)
        } else {
            Ok(())
        }
    }

    /// Refuses to start a run or recovery on a session that owns one or is poisoned.
    fn idle<E: std::error::Error>(&self) -> Result<(), Error<E>> {
        if self.owner.is_some() || self.poisoned {
            return Err(Error::History(
                "session already owns a run or is not clean".into(),
            ));
        }

        Ok(())
    }

    /// Records that an autocommit body sends a statement, before sending it:
    /// a send that fails has an unknown outcome.
    pub fn dispatch(&mut self) {
        if let Some(Body::Autocommit { dispatched }) = &mut self.body {
            *dispatched = true;
        }
    }

    /// Whether user statements may run: owned, unpoisoned, inside a begun attempt body.
    pub fn user_state(&self) -> Result<(), StateError> {
        if self.owner.is_none() {
            Err(StateError::NotOwner)
        } else if self.poisoned {
            Err(StateError::Poisoned)
        } else if self.attempt.is_none() {
            Err(StateError::NoAttempt)
        } else if self.body.is_none() {
            Err(StateError::NoTransaction)
        } else {
            Ok(())
        }
    }

    /// Admits a typed user statement rendered as `sql`. PostgreSQL has no
    /// client-side authorizer, so the script grammar refuses it instead.
    pub fn user_statement(&mut self, sql: &str) -> Result<(), StateError> {
        self.user_state()?;
        crate::validate_script(sql, DialectId::Postgres).map_err(StateError::Script)?;
        self.dispatch();

        Ok(())
    }

    /// The `lock_timeout` in milliseconds, at least one, clamped to PostgreSQL's maximum.
    fn lock_millis(&self) -> u128 {
        self.lock_timeout.as_millis().clamp(1, MAX_LOCK_TIMEOUT_MS)
    }

    /// The owned namespace, unquoted.
    fn owned_namespace<E: std::error::Error>(&self) -> Result<String, Error<E>> {
        self.owner
            .as_ref()
            .map(|owner| owner.namespace.clone())
            .ok_or_else(|| Error::History("migration connection does not own the target".into()))
    }

    /// The qualified journal of the owned namespace.
    fn journal<E: std::error::Error>(&self) -> Result<String, Error<E>> {
        Ok(journal(&self.owned_namespace()?))
    }
}

/// Marks the session unusable after a run ended without returning.
pub fn abandon(state: &mut State) {
    state.poisoned = true;
}

/// What a clean migration connection reports about itself.
#[derive(Debug)]
struct Connection {
    database: String,
    namespace: String,
}

impl Connection {
    /// Reads the [`sql::DESCRIBE`] row and the second [`sql::TRANSACTION_START`],
    /// rejecting connections inside an application transaction or with
    /// settings the script grammar does not assume.
    fn describe<E: std::error::Error>(
        row: Vec<Value>,
        again: Option<String>,
    ) -> Result<Self, Error<E>> {
        let Ok(
            [
                Value::Text(started),
                standard,
                Value::Text(database),
                namespace,
            ],
        ) = <[Value; 4]>::try_from(row)
        else {
            return Err(Error::History(
                "connection did not describe its database".into(),
            ));
        };

        if again.as_deref() == Some(started.as_str()) {
            return Err(Error::History(
                "connection is already inside an application transaction".into(),
            ));
        }

        if super::text(standard).as_deref() != Some("on") {
            return Err(Error::History(
                "migration connections require standard_conforming_strings = on".into(),
            ));
        }

        let Some(namespace) = super::text(namespace) else {
            return Err(Error::History(
                "migration connections require a current schema on the search_path".into(),
            ));
        };

        Ok(Self {
            database,
            namespace,
        })
    }

    /// The advisory lock key of this target.
    fn key(&self) -> i64 {
        Target {
            engine: Engine::Postgres,
            database: &self.database,
            namespace: &self.namespace,
        }
        .advisory_key()
    }
}

/// A bigint advisory key as `pg_locks` reports it: high and low 32 bits.
fn lock_parts(key: i64) -> [Value; 2] {
    [
        Value::I64(i64::from((key >> 32) as u32)),
        Value::I64(i64::from(key as u32)),
    ]
}

/// The journal of `namespace`, qualified for interpolation into framework-owned SQL.
fn journal(namespace: &str) -> String {
    format!("{}.{}", quote(namespace), sql::JOURNAL)
}

/// Quotes an identifier for interpolation into framework-owned SQL.
fn quote(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

#[cfg(feature = "sync")]
steps::steps!(sync, [] []);
#[cfg(feature = "async")]
steps::steps!(asynchronous, [async] [.await]);
