//! The SQLite migration session protocol. SQLite has no session locks, so a
//! runner claims the target durably in the journal's init row under a
//! `BEGIN IMMEDIATE` writer lock, and each body runs in its own immediate
//! transaction. User statements run under [`authorize`], installed through
//! the driver's `SqliteIo` (in `protocol::sync` or `protocol::asynchronous`).

mod inspect;
mod recovery;
pub mod sql;
mod steps;

use std::time::Duration;

use crate::{Engine, Policy, StateError};

/// The policies SQLite sessions honour.
pub const POLICIES: &[Policy] = Engine::Sqlite.policies();

/// Per-connection ownership and attempt state.
#[derive(Debug)]
pub struct State {
    owner: Option<String>,
    /// The connection's own `busy_timeout`, restored at release.
    busy_timeout: Option<i64>,
    attempt: Option<i64>,
    poisoned: bool,
    /// How long to wait for SQLite's writer lock.
    pub lock_timeout: Duration,
}

impl Default for State {
    fn default() -> Self {
        Self {
            owner: None,
            busy_timeout: None,
            attempt: None,
            poisoned: false,
            lock_timeout: Duration::from_secs(5),
        }
    }
}

impl State {
    /// Whether a run left state on the connection: owned, mid-attempt or
    /// poisoned. Pools must discard such connections.
    pub fn is_active(&self) -> bool {
        self.owner.is_some() || self.attempt.is_some() || self.poisoned
    }

    /// Whether statements route through the authorized user path.
    pub fn is_running(&self) -> bool {
        self.owner.is_some() || self.poisoned
    }

    /// Whether user statements may run, apart from the open transaction.
    fn user_state(&self) -> Result<(), StateError> {
        if self.owner.is_none() {
            Err(StateError::NotOwner)
        } else if self.poisoned {
            Err(StateError::Poisoned)
        } else if self.attempt.is_none() {
            Err(StateError::NoAttempt)
        } else {
            Ok(())
        }
    }
}

/// Marks the session unusable after a run ended without returning.
pub fn abandon(state: &mut State) {
    state.poisoned = true;
}

/// SQLite authorizer action codes (`sqlite3.h`) the policy can allow.
pub mod action {
    /// `SQLITE_CREATE_INDEX`: index, table.
    pub const CREATE_INDEX: i32 = 1;
    /// `SQLITE_CREATE_TABLE`: table.
    pub const CREATE_TABLE: i32 = 2;
    /// `SQLITE_CREATE_TRIGGER`: trigger, table.
    pub const CREATE_TRIGGER: i32 = 7;
    /// `SQLITE_CREATE_VIEW`: view.
    pub const CREATE_VIEW: i32 = 8;
    /// `SQLITE_DELETE`: table.
    pub const DELETE: i32 = 9;
    /// `SQLITE_DROP_INDEX`: index, table.
    pub const DROP_INDEX: i32 = 10;
    /// `SQLITE_DROP_TABLE`: table.
    pub const DROP_TABLE: i32 = 11;
    /// `SQLITE_DROP_TRIGGER`: trigger, table.
    pub const DROP_TRIGGER: i32 = 16;
    /// `SQLITE_DROP_VIEW`: view.
    pub const DROP_VIEW: i32 = 17;
    /// `SQLITE_INSERT`: table.
    pub const INSERT: i32 = 18;
    /// `SQLITE_READ`: table, column.
    pub const READ: i32 = 20;
    /// `SQLITE_SELECT`.
    pub const SELECT: i32 = 21;
    /// `SQLITE_UPDATE`: table, column.
    pub const UPDATE: i32 = 23;
    /// `SQLITE_ALTER_TABLE`: database, table.
    pub const ALTER_TABLE: i32 = 26;
    /// `SQLITE_REINDEX`: index.
    pub const REINDEX: i32 = 27;
    /// `SQLITE_ANALYZE`: table.
    pub const ANALYZE: i32 = 28;
    /// `SQLITE_FUNCTION`: (none), function.
    pub const FUNCTION: i32 = 31;
    /// `SQLITE_RECURSIVE`.
    pub const RECURSIVE: i32 = 33;
}

/// Whether a user statement may perform authorizer action `code` with its
/// `first`/`second` arguments on `database`: only `main`, never a reserved
/// `_rupa_` object, no transaction control, attachment, pragma or extension
/// loading. Shared by every SQLite driver so they refuse the same statements.
///
/// `ALTER TABLE` is the exception to the argument layout: SQLite passes its
/// database as `first` and the dropped column, if any, in the `database`
/// slot, so its namespace is checked from `first`.
pub fn authorize(
    code: i32,
    first: Option<&str>,
    second: Option<&str>,
    database: Option<&str>,
) -> bool {
    let open = |name: Option<&str>| name.is_some_and(|name| !reserved(name));

    if code == action::ALTER_TABLE {
        return first == Some("main") && open(second);
    }

    if database == Some("temp") {
        return matches!(code, action::READ | action::UPDATE)
            && matches!(first, Some("sqlite_temp_master" | "sqlite_temp_schema"));
    }

    if database.is_some_and(|name| name != "main") {
        return false;
    }

    match code {
        action::CREATE_TABLE
        | action::DROP_TABLE
        | action::INSERT
        | action::DELETE
        | action::UPDATE
        | action::READ
        | action::ANALYZE
        | action::CREATE_VIEW
        | action::DROP_VIEW
        | action::REINDEX => open(first),
        action::CREATE_INDEX
        | action::DROP_INDEX
        | action::CREATE_TRIGGER
        | action::DROP_TRIGGER => open(first) && open(second),
        action::FUNCTION => second.is_some_and(|name| !name.eq_ignore_ascii_case("load_extension")),
        action::SELECT | action::RECURSIVE => true,
        _ => false,
    }
}

fn reserved(name: &str) -> bool {
    name.to_ascii_lowercase().starts_with("_rupa_")
}

#[cfg(feature = "sync")]
steps::steps!(sync, [] []);
#[cfg(feature = "async")]
steps::steps!(asynchronous, [async] [.await]);

#[cfg(test)]
mod tests;
