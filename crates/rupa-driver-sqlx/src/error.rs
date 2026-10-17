use std::error::Error as StdError;
use std::fmt;

use rupa_core::exec::ExecError;
use rupa_core::{ResultError, TxError};
use rupa_sql::RenderError;

/// Everything that can go wrong running a statement through sqlx.
#[derive(Debug)]
#[non_exhaustive]
pub enum SqlxError {
    /// The statement could not be rendered in the database's dialect.
    Render(RenderError),
    /// sqlx (or the database) reported an error.
    Db(sqlx::Error),
    /// The rows did not fit the query's result type.
    Result(ResultError),
    /// The transaction API was misused.
    Tx(TxError),
    /// No pooled connection could be obtained (e.g. a timeout).
    Pool(Box<dyn StdError + Send + Sync>),
}

impl SqlxError {
    /// The sqlx error, if the database or the connection failed.
    pub fn as_sqlx(&self) -> Option<&sqlx::Error> {
        match self {
            SqlxError::Db(e) => Some(e),
            _ => None,
        }
    }

    /// Whether the connection is unusable after this error (I/O, TLS or
    /// protocol failures), so a pool must not hand it out again.
    pub fn is_fatal(&self) -> bool {
        matches!(
            self,
            SqlxError::Db(
                sqlx::Error::Io(_)
                    | sqlx::Error::Tls(_)
                    | sqlx::Error::Protocol(_)
                    | sqlx::Error::WorkerCrashed
            )
        )
    }
}

impl fmt::Display for SqlxError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SqlxError::Render(e) => write!(f, "rendering failed: {e}"),
            SqlxError::Db(e) => e.fmt(f),
            SqlxError::Result(e) => e.fmt(f),
            SqlxError::Tx(e) => e.fmt(f),
            SqlxError::Pool(e) => write!(f, "connection pool: {e}"),
        }
    }
}

impl StdError for SqlxError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            SqlxError::Render(e) => Some(e),
            SqlxError::Db(e) => Some(e),
            SqlxError::Result(e) => Some(e),
            SqlxError::Tx(e) => Some(e),
            SqlxError::Pool(e) => Some(&**e),
        }
    }
}

impl From<RenderError> for SqlxError {
    fn from(e: RenderError) -> Self {
        SqlxError::Render(e)
    }
}

impl From<sqlx::Error> for SqlxError {
    fn from(e: sqlx::Error) -> Self {
        SqlxError::Db(e)
    }
}

impl From<TxError> for SqlxError {
    fn from(e: TxError) -> Self {
        SqlxError::Tx(e)
    }
}

impl From<ResultError> for SqlxError {
    fn from(e: ResultError) -> Self {
        SqlxError::Result(e)
    }
}

impl ExecError for SqlxError {
    fn result_error(&self) -> Option<&ResultError> {
        match self {
            SqlxError::Result(e) => Some(e),
            _ => None,
        }
    }
}

impl From<SqlxError> for rupa_core::DynError {
    fn from(e: SqlxError) -> Self {
        rupa_core::DynError::new(e)
    }
}
