use std::error::Error as StdError;
use std::fmt;

use crate::dialect::{Capability, DialectId};
use crate::exec::ExecError;
use crate::value::{SqlType, Value};

/// A value could not be decoded into the requested Rust type.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodeError {
    /// The column being decoded, when known.
    pub column: Option<&'static str>,
    /// What went wrong.
    pub message: String,
}

impl DecodeError {
    /// An error with `message` and no column yet.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            column: None,
            message: message.into(),
        }
    }

    /// `got` is not a value of the `expected` type.
    pub fn type_mismatch(expected: SqlType, got: &Value) -> Self {
        Self::new(format!("expected {expected:?}, got {got:?}"))
    }

    /// Attributes the error to `column`, unless it already names one.
    pub fn in_column(mut self, column: &'static str) -> Self {
        self.column.get_or_insert(column);
        self
    }
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(column) = self.column {
            write!(f, "column `{column}`: ")?;
        }

        f.write_str(&self.message)
    }
}

impl std::error::Error for DecodeError {}

/// A driver could not produce a value from a row.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum RowError {
    /// The row has fewer columns than the query projected.
    #[error("column index {index} out of range (row has {len} columns)")]
    IndexOutOfRange {
        /// The requested column.
        index: usize,
        /// The row's width.
        len: usize,
    },
    /// The driver could not read the value.
    #[error("{0}")]
    Driver(String),
}

impl RowError {
    /// Values of `ty` need a cargo feature (`chrono`, `uuid`) that is not
    /// enabled.
    pub fn not_enabled(ty: SqlType) -> Self {
        RowError::Driver(format!(
            "{ty:?} values need the `{}` feature",
            ty.feature().unwrap_or("?")
        ))
    }
}

/// Rows or an affected-row count could not be shaped into the query's result type.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum ResultError {
    /// `T` expected exactly one row; none matched.
    #[error("expected exactly one row, found none")]
    NotFound,
    /// `T` or `Option<T>` expected at most one row; more matched.
    #[error("expected at most one row, found more")]
    TooManyRows,
    /// The executor produced rows where a count was expected, or vice versa.
    #[error("executor output does not match the query's result shape")]
    ShapeMismatch,
    /// A column could not be read.
    #[error(transparent)]
    Row(#[from] RowError),
    /// A column could not be decoded into its field type.
    #[error(transparent)]
    Decode(#[from] DecodeError),
}

/// Errors raised by a DSL function's lowering or evaluation.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum DslError {
    /// The function has no lowering for the dialect.
    #[error("`{function}` is not supported on dialect {dialect:?}")]
    Unsupported {
        /// The function's name.
        function: &'static str,
        /// The dialect it was rendered for.
        dialect: DialectId,
    },
    /// The function was called with the wrong number of arguments.
    #[error("`{function}` takes {expected} arguments, got {got}")]
    Arity {
        /// The function's name.
        function: &'static str,
        /// The number of arguments it takes.
        expected: usize,
        /// The number it was given.
        got: usize,
    },
    /// Any other failure.
    #[error("{0}")]
    Other(String),
}

impl DslError {
    /// `function` has no lowering for `dialect`.
    pub fn unsupported(function: &'static str, dialect: DialectId) -> Self {
        DslError::Unsupported { function, dialect }
    }
}

/// Misuse of the transaction API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum TxError {
    /// A repository tried to use a transaction after commit or rollback.
    #[error("transaction context is closed")]
    Closed,
    /// Isolation level and read-only can only be set on a top-level
    /// transaction; nested transactions are savepoints.
    #[error("transaction options (isolation, read-only) cannot be set on a nested transaction")]
    OptionsOnNested,
    /// A transaction or savepoint is already open at this depth on the
    /// connection; open a nested one on the innermost transaction instead.
    #[error("a transaction is already open at this depth; nest inside the innermost one")]
    AlreadyOpen,
    /// The dialect lacks a capability the transaction needs, e.g.
    /// `NativeRls` for `try_begin_with_security`.
    #[error("dialect {dialect:?} does not support {capability:?}")]
    Unsupported {
        /// The executor's dialect.
        dialect: DialectId,
        /// The missing capability.
        capability: Capability,
    },
}

/// Why a [`DbConfig`](crate::config::DbConfig) could not be turned into a
/// connection pool.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConnectError {
    /// `driver` names no registered driver.
    #[error(
        "no driver named `{name}` is registered (registered: {})",
        registered(available)
    )]
    UnknownDriver {
        /// The requested name.
        name: String,
        /// Registered drivers, as `name (schemes)`.
        available: Vec<String>,
    },
    /// No registered driver accepts the URL's scheme.
    #[error(
        "no registered driver accepts `{scheme}:` URLs (registered: {})",
        registered(available)
    )]
    UnknownScheme {
        /// The URL's scheme.
        scheme: String,
        /// Registered drivers, as `name (schemes)`.
        available: Vec<String>,
    },
    /// The URL is malformed or doesn't fit the driver.
    #[error("invalid database URL: {0}")]
    InvalidUrl(String),
    /// A pool setting or driver option is invalid or unknown.
    #[error("invalid database option `{key}`: {message}")]
    InvalidOption {
        /// The option's key.
        key: String,
        /// What is wrong with it.
        message: String,
    },
    /// The driver could not open its first connection.
    #[error("could not connect: {0}")]
    Connect(#[source] DynError),
}

fn registered(available: &[String]) -> String {
    if available.is_empty() {
        return "none; enable a driver feature of `rupa` (sqlite, postgres, mysql, ..)".to_owned();
    }

    available.join(", ")
}

/// Error of an erased executor: the driver's error, boxed, plus its
/// [`ResultError`] if it was one, so `NotFound` & co. stay inspectable.
#[derive(Debug)]
pub struct DynError {
    inner: Box<dyn StdError + Send + Sync>,
    result: Option<ResultError>,
}

impl DynError {
    /// Wraps a driver error.
    pub fn new<E: ExecError>(e: E) -> Self {
        let result = e.result_error().cloned();
        Self {
            inner: Box::new(e),
            result,
        }
    }

    /// Wraps any error that is not an executor error (a pool timeout, a
    /// configuration problem).
    pub fn other(e: impl Into<Box<dyn StdError + Send + Sync>>) -> Self {
        Self {
            inner: e.into(),
            result: None,
        }
    }

    /// The wrapped error, e.g. to downcast to the driver's error type.
    pub fn into_inner(self) -> Box<dyn StdError + Send + Sync> {
        self.inner
    }

    /// The wrapped error, by reference.
    pub fn get_ref(&self) -> &(dyn StdError + Send + Sync + 'static) {
        &*self.inner
    }

    /// The wrapped error as `T`, if it is one.
    pub fn downcast_ref<T: StdError + 'static>(&self) -> Option<&T> {
        self.inner.downcast_ref()
    }
}

impl fmt::Display for DynError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.inner.fmt(f)
    }
}

impl StdError for DynError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(&*self.inner)
    }
}

impl From<ResultError> for DynError {
    fn from(e: ResultError) -> Self {
        Self {
            inner: Box::new(e.clone()),
            result: Some(e),
        }
    }
}

impl From<TxError> for DynError {
    fn from(e: TxError) -> Self {
        Self::other(e)
    }
}

impl ExecError for DynError {
    fn result_error(&self) -> Option<&ResultError> {
        self.result.as_ref()
    }

    fn into_dyn_error(self) -> DynError {
        self
    }
}
