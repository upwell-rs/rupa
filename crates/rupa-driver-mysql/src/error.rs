//! The driver's error type.

use std::error::Error as StdError;
use std::fmt;

use rupa_core::exec::ExecError;
use rupa_core::{DynError, ResultError, TxError};
use rupa_sql::RenderError;

/// Everything that can go wrong running a statement on MySQL.
#[derive(Debug)]
#[non_exhaustive]
pub enum MySqlError {
    /// The statement could not be rendered as MySQL SQL.
    Render(RenderError),
    /// MySQL reported an error.
    Db(mysql::Error),
    /// The rows did not fit the query's result type.
    Result(ResultError),
    /// The transaction API was misused.
    Tx(TxError),
    /// No pooled connection could be obtained (e.g. a timeout).
    Pool(Box<dyn StdError + Send + Sync>),
}

impl fmt::Display for MySqlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MySqlError::Render(e) => write!(f, "rendering failed: {e}"),
            MySqlError::Db(e) => e.fmt(f),
            MySqlError::Result(e) => e.fmt(f),
            MySqlError::Tx(e) => e.fmt(f),
            MySqlError::Pool(e) => write!(f, "connection pool: {e}"),
        }
    }
}

impl StdError for MySqlError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            MySqlError::Render(e) => Some(e),
            MySqlError::Db(e) => Some(e),
            MySqlError::Result(e) => Some(e),
            MySqlError::Tx(e) => Some(e),
            MySqlError::Pool(e) => Some(&**e),
        }
    }
}

impl From<RenderError> for MySqlError {
    fn from(e: RenderError) -> Self {
        MySqlError::Render(e)
    }
}

impl From<mysql::Error> for MySqlError {
    fn from(e: mysql::Error) -> Self {
        MySqlError::Db(e)
    }
}

impl From<ResultError> for MySqlError {
    fn from(e: ResultError) -> Self {
        MySqlError::Result(e)
    }
}

impl From<TxError> for MySqlError {
    fn from(e: TxError) -> Self {
        MySqlError::Tx(e)
    }
}

impl ExecError for MySqlError {
    fn result_error(&self) -> Option<&ResultError> {
        match self {
            MySqlError::Result(e) => Some(e),
            _ => None,
        }
    }
}

impl From<MySqlError> for DynError {
    fn from(e: MySqlError) -> Self {
        DynError::new(e)
    }
}
