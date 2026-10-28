use std::ffi::{CStr, c_char, c_int, c_void};
use std::ptr;
use std::time::Duration;

use libsqlite3_sys::{SQLITE_DENY, SQLITE_OK, sqlite3_get_autocommit, sqlite3_set_authorizer};
use rupa_core::exec::Outcome;
use rupa_core::ir::Statement;
use rupa_core::query::Expect;
use rupa_migrate::StateError;
use rupa_migrate::protocol::asynchronous::SqliteIo;
use rupa_migrate::protocol::sqlite::{self, State, authorize};
use sqlx::Connection as _;

use super::Engine;
use crate::{SqlxError, SqlxExecutor};

rupa_migrate::__session!(async, sqlite, SqlxExecutor<sqlx::Sqlite>);

/// Typed statements run under the shared migration authorizer.
impl Engine<sqlx::Sqlite> for State {
    fn is_active(&self) -> bool {
        State::is_active(self)
    }

    fn outside_run(&self) -> Result<(), StateError> {
        State::outside_run(self)
    }

    fn lock_timeout(&mut self) -> &mut Duration {
        &mut self.lock_timeout
    }

    async fn user_statement<'e>(
        executor: &'e mut SqlxExecutor<sqlx::Sqlite>,
        statement: &'e Statement,
        expect: Expect,
    ) -> Result<Outcome, SqlxError> {
        sqlite::asynchronous::user_begin(executor).await?;

        let result = executor.dispatch(statement, expect).await;
        let ended = sqlite::asynchronous::user_end(executor).await;

        result.and_then(|outcome| ended.map(|()| outcome))
    }
}

/// sqlx exposes neither the autocommit flag nor an authorizer, so both use
/// the raw handle, locked away from sqlx's worker thread for the call.
impl SqliteIo for SqlxExecutor<sqlx::Sqlite> {
    async fn is_autocommit(&mut self) -> Result<bool, SqlxError> {
        let autocommit = match self.conn.get_mut().lock_handle().await {
            // SAFETY: the locked handle is a live connection no other thread
            // uses until it is dropped, and the call only reads its state.
            Ok(mut handle) => {
                Ok(unsafe { sqlite3_get_autocommit(handle.as_raw_handle().as_ptr()) })
            }
            Err(error) => Err(error),
        };

        autocommit.map(|flag| flag != 0).map_err(|e| self.failed(e))
    }

    /// Prepared statements were authorized under the previous policy, so the
    /// statement cache is cleared first.
    async fn authorizer(&mut self, enabled: bool) -> Result<(), SqlxError> {
        let cleared = self.conn.get_mut().clear_cached_statements().await;

        cleared.map_err(|e| self.failed(e))?;

        let callback = enabled.then_some(decide as Callback);
        let installed = match self.conn.get_mut().lock_handle().await {
            // SAFETY: the locked handle is a live connection no other thread
            // uses until it is dropped. `decide` keeps no state, never unwinds
            // and reads its arguments only for the duration of each call, and
            // no user data pointer is registered.
            Ok(mut handle) => Ok(unsafe {
                sqlite3_set_authorizer(handle.as_raw_handle().as_ptr(), callback, ptr::null_mut())
            }),
            Err(error) => Err(error),
        };
        let code = installed.map_err(|e| self.failed(e))?;

        if code != SQLITE_OK {
            return Err(SqlxError::Db(sqlx::Error::Protocol(format!(
                "sqlite3_set_authorizer failed with code {code}"
            ))));
        }

        Ok(())
    }
}

/// The C signature of an SQLite authorizer.
type Callback = unsafe extern "C" fn(
    *mut c_void,
    c_int,
    *const c_char,
    *const c_char,
    *const c_char,
    *const c_char,
) -> c_int;

/// Applies the shared migration policy to one authorizer request. Arguments
/// that are not UTF-8 read as absent, which the policy denies.
unsafe extern "C" fn decide(
    _: *mut c_void,
    code: c_int,
    first: *const c_char,
    second: *const c_char,
    database: *const c_char,
    _: *const c_char,
) -> c_int {
    let text = |pointer: *const c_char| {
        // SAFETY: SQLite passes each argument as null or a NUL-terminated
        // string that stays valid for the duration of the callback.
        (!pointer.is_null())
            .then(|| unsafe { CStr::from_ptr(pointer) }.to_str().ok())
            .flatten()
    };

    if authorize(code, text(first), text(second), text(database)) {
        SQLITE_OK
    } else {
        SQLITE_DENY
    }
}
