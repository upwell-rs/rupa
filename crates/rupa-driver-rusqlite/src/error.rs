//! The driver's error type.

use std::error::Error as StdError;
use std::fmt;

use rupa_core::exec::ExecError;
use rupa_core::{DynError, ResultError, TxError};
use rupa_sql::RenderError;

/// Everything that can go wrong running a statement on SQLite.
#[derive(Debug)]
#[non_exhaustive]
pub enum SqliteError {
    /// The statement could not be rendered as SQLite SQL.
    Render(RenderError),
    /// SQLite reported an error.
    Db(rusqlite::Error),
    /// The rows did not fit the query's result type.
    Result(ResultError),
    /// The transaction API was misused.
    Tx(TxError),
    /// No pooled connection could be obtained (e.g. a timeout).
    Pool(Box<dyn StdError + Send + Sync>),
}

impl fmt::Display for SqliteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SqliteError::Render(e) => write!(f, "rendering failed: {e}"),
            SqliteError::Db(e) => e.fmt(f),
            SqliteError::Result(e) => e.fmt(f),
            SqliteError::Tx(e) => e.fmt(f),
            SqliteError::Pool(e) => write!(f, "connection pool: {e}"),
        }
    }
}

impl StdError for SqliteError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            SqliteError::Render(e) => Some(e),
            SqliteError::Db(e) => Some(e),
            SqliteError::Result(e) => Some(e),
            SqliteError::Tx(e) => Some(e),
            SqliteError::Pool(e) => Some(&**e),
        }
    }
}

impl From<RenderError> for SqliteError {
    fn from(e: RenderError) -> Self {
        SqliteError::Render(e)
    }
}

impl From<rusqlite::Error> for SqliteError {
    fn from(e: rusqlite::Error) -> Self {
        SqliteError::Db(e)
    }
}

impl From<ResultError> for SqliteError {
    fn from(e: ResultError) -> Self {
        SqliteError::Result(e)
    }
}

impl From<TxError> for SqliteError {
    fn from(e: TxError) -> Self {
        SqliteError::Tx(e)
    }
}

impl ExecError for SqliteError {
    fn result_error(&self) -> Option<&ResultError> {
        match self {
            SqliteError::Result(e) => Some(e),
            _ => None,
        }
    }
}

impl From<SqliteError> for DynError {
    fn from(e: SqliteError) -> Self {
        DynError::new(e)
    }
}
