use std::fmt;

use crate::dialect::DialectId;
use crate::value::{SqlType, Value};

/// A value could not be decoded into the requested Rust type.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodeError {
    pub column: Option<&'static str>,
    pub message: String,
}

impl DecodeError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            column: None,
            message: message.into(),
        }
    }

    pub fn type_mismatch(expected: SqlType, got: &Value) -> Self {
        Self::new(format!("expected {expected:?}, got {got:?}"))
    }

    pub fn in_column(mut self, column: &'static str) -> Self {
        self.column.get_or_insert(column);
        self
    }
}

impl fmt::Display for DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.column {
            Some(c) => write!(f, "column `{c}`: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for DecodeError {}

/// A driver could not produce a value from a row.
#[derive(Debug, Clone, PartialEq)]
pub enum RowError {
    IndexOutOfRange { index: usize, len: usize },
    Driver(String),
}

impl fmt::Display for RowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RowError::IndexOutOfRange { index, len } => {
                write!(
                    f,
                    "column index {index} out of range (row has {len} columns)"
                )
            }
            RowError::Driver(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for RowError {}

/// Rows or an affected-row count could not be shaped into the query's result type.
#[derive(Debug, Clone, PartialEq)]
pub enum ResultError {
    /// `T` expected exactly one row; none matched.
    NotFound,
    /// `T` or `Option<T>` expected at most one row; more matched.
    TooManyRows,
    /// The executor produced rows where a count was expected, or vice versa.
    ShapeMismatch,
    Row(RowError),
    Decode(DecodeError),
}

impl From<RowError> for ResultError {
    fn from(e: RowError) -> Self {
        ResultError::Row(e)
    }
}

impl From<DecodeError> for ResultError {
    fn from(e: DecodeError) -> Self {
        ResultError::Decode(e)
    }
}

impl fmt::Display for ResultError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ResultError::NotFound => f.write_str("expected exactly one row, found none"),
            ResultError::TooManyRows => f.write_str("expected at most one row, found more"),
            ResultError::ShapeMismatch => {
                f.write_str("executor output does not match the query's result shape")
            }
            ResultError::Row(e) => e.fmt(f),
            ResultError::Decode(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for ResultError {}

/// Errors raised by a DSL function's lowering or evaluation.
#[derive(Debug, Clone, PartialEq)]
pub enum DslError {
    Unsupported {
        function: &'static str,
        dialect: DialectId,
    },
    Arity {
        function: &'static str,
        expected: usize,
        got: usize,
    },
    Other(String),
}

impl DslError {
    pub fn unsupported(function: &'static str, dialect: DialectId) -> Self {
        DslError::Unsupported { function, dialect }
    }
}

impl fmt::Display for DslError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DslError::Unsupported { function, dialect } => {
                write!(f, "`{function}` is not supported on dialect {dialect:?}")
            }
            DslError::Arity {
                function,
                expected,
                got,
            } => {
                write!(f, "`{function}` takes {expected} arguments, got {got}")
            }
            DslError::Other(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for DslError {}
