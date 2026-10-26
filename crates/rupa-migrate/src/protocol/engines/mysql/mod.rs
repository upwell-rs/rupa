//! The MySQL migration session protocol. MySQL commits DDL implicitly, so
//! sessions run nontransactional bodies only; a named session lock keyed by
//! [`Target::lock_name`] keeps runners exclusive across the per-artifact
//! commits, and the InnoDB journal table lives in the selected database.

mod coverage;
mod inspect;
mod recovery;
pub mod sql;
mod steps;

use std::time::Duration;

use rupa_core::{DialectId, Value};

use crate::{Engine, Error, Policy, StateError, Target};

/// The policies MySQL sessions honour.
pub const POLICIES: &[Policy] = Engine::MySql.policies();

/// The `sql_mode` flag the migration script grammar assumes.
const NO_BACKSLASH_ESCAPES: &str = "NO_BACKSLASH_ESCAPES";

/// `sql_mode` flags that read `"…"` as an identifier, which the script
/// grammar reads as a string. `@@SESSION.sql_mode` lists the composite `ANSI`
/// beside the flags it expands to, so both are removed.
const IDENTIFIER_QUOTES: [&str; 2] = ["ANSI_QUOTES", "ANSI"];

/// The named lock a connection holds and the session state it replaced.
#[derive(Debug)]
struct Owner {
    lock: String,
    sql_mode: String,
}

/// Per-connection ownership, attempt and body state.
#[derive(Debug)]
pub struct State {
    owner: Option<Owner>,
    attempt: Option<i64>,
    body: bool,
    dispatched: bool,
    poisoned: bool,
    /// How long to wait for another runner's lock.
    pub lock_timeout: Duration,
}

impl Default for State {
    fn default() -> Self {
        Self {
            owner: None,
            attempt: None,
            body: false,
            dispatched: false,
            poisoned: false,
            lock_timeout: Duration::from_secs(5),
        }
    }
}

impl State {
    /// Whether a run left state on the connection: owned, mid-attempt,
    /// mid-body or poisoned. Pools must discard such connections.
    pub fn is_active(&self) -> bool {
        self.owner.is_some() || self.attempt.is_some() || self.body || self.poisoned
    }

    /// Whether statements route through [`State::user_statement`].
    pub fn is_running(&self) -> bool {
        self.owner.is_some() || self.poisoned
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

    /// Whether user statements may run: owned, unpoisoned, inside a begun attempt body.
    fn user_state(&self) -> Result<(), StateError> {
        if self.owner.is_none() {
            Err(StateError::NotOwner)
        } else if self.poisoned {
            Err(StateError::Poisoned)
        } else if self.attempt.is_none() {
            Err(StateError::NoAttempt)
        } else if !self.body {
            Err(StateError::NoTransaction)
        } else {
            Ok(())
        }
    }

    /// Admits a typed user statement rendered as `sql`. MySQL has no
    /// client-side authorizer, so the script grammar refuses it instead.
    pub fn user_statement(&mut self, sql: &str) -> Result<(), StateError> {
        self.user_state()?;
        crate::validate_script(sql, DialectId::MySql).map_err(StateError::Script)?;
        self.dispatched = true;

        Ok(())
    }

    /// `GET_LOCK` waits whole seconds: a nonzero timeout rounds up, zero tries once.
    fn lock_seconds(&self) -> i64 {
        let seconds = self
            .lock_timeout
            .as_secs()
            .saturating_add(u64::from(self.lock_timeout.subsec_nanos() > 0));

        i64::try_from(seconds).unwrap_or(i64::MAX)
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
    sql_mode: String,
}

impl Connection {
    /// The named lock of this target: the selected database is both the
    /// database and the owned namespace.
    fn lock(&self) -> String {
        Target {
            engine: Engine::MySql,
            database: &self.database,
            namespace: &self.database,
        }
        .lock_name()
    }

    /// The session `sql_mode` with `NO_BACKSLASH_ESCAPES` added and the
    /// [`IDENTIFIER_QUOTES`] flags removed, so the server reads scripts as
    /// the grammar does.
    fn migration_sql_mode(&self) -> String {
        let mut modes: Vec<&str> = self
            .sql_mode
            .split(',')
            .filter(|mode| {
                !mode.is_empty()
                    && !mode.eq_ignore_ascii_case(NO_BACKSLASH_ESCAPES)
                    && !IDENTIFIER_QUOTES
                        .iter()
                        .any(|quotes| mode.eq_ignore_ascii_case(quotes))
            })
            .collect();

        modes.push(NO_BACKSLASH_ESCAPES);

        modes.join(",")
    }
}

/// `value` as text.
fn bound(value: &str) -> Value {
    Value::Text(value.to_owned())
}

/// The parameters of [`sql::FINISH`]: the shared ones, with the attempt id last.
fn finish(attempt: i64, status: &str, recovery: Option<&crate::Recovery<'_>>) -> [Value; 4] {
    let mut params = crate::protocol::finish(attempt, status, recovery);

    params.rotate_left(1);

    params
}

/// The failure `GET_LOCK` or `RELEASE_LOCK` reports as `NULL`.
fn lock_failed<E: std::error::Error>(what: &str) -> Error<E> {
    Error::History(format!("{what} failed for the migration lock"))
}

#[cfg(feature = "sync")]
steps::steps!(sync, [] []);
#[cfg(feature = "async")]
steps::steps!(asynchronous, [async] [.await]);

#[cfg(test)]
mod tests;
