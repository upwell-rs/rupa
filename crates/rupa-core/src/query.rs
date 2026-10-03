//! `Query<R>`: a built, dialect-free statement plus the result type `R` it yields.
//!
//! Result shapes split into two families so the shape always matches the
//! statement kind:
//! - [`RowsResult`] (`T`, `Option<T>`, `Vec<T>` with `T: FromRow`): selects.
//! - [`AffectedResult`] (`u64`, `bool`): insert / update / delete.
//!
//! `bool` also serves `exists()` selects, which yield one boolean row.

use std::fmt;
use std::marker::PhantomData;

use crate::entity::FromRow;
use crate::error::ResultError;
use crate::ir::Statement;
use crate::row::RowCursor;
use crate::value::{ScalarColumn, SqlType};

/// What the executor should expect back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Expect {
    Many,
    /// Zero or one row; more is [`ResultError::TooManyRows`].
    AtMostOne,
    /// Exactly one row.
    ExactlyOne,
    /// An affected-row count.
    Affected,
    /// One row with one boolean column.
    Exists,
}

impl Expect {
    /// Rows an executor needs to fetch to decide the result (`None` = all).
    /// `AtMostOne`/`ExactlyOne` need a second row only to detect "too many".
    pub fn fetch_limit(self) -> Option<usize> {
        match self {
            Expect::Many => None,
            Expect::AtMostOne | Expect::ExactlyOne => Some(2),
            Expect::Exists => Some(1),
            Expect::Affected => Some(0),
        }
    }
}

pub struct Query<R> {
    statement: Statement,
    expect: Expect,
    _r: PhantomData<fn() -> R>,
}

impl<R> Clone for Query<R> {
    fn clone(&self) -> Self {
        Self::new(self.statement.clone(), self.expect)
    }
}

impl<R> fmt::Debug for Query<R> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Query")
            .field("result", &std::any::type_name::<R>())
            .field("expect", &self.expect)
            .field("statement", &self.statement)
            .finish()
    }
}

impl<R> Query<R> {
    pub(crate) fn new(statement: Statement, expect: Expect) -> Self {
        Self {
            statement,
            expect,
            _r: PhantomData,
        }
    }

    pub fn statement(&self) -> &Statement {
        &self.statement
    }

    pub fn into_statement(self) -> Statement {
        self.statement
    }

    pub fn expect(&self) -> Expect {
        self.expect
    }
}

impl<R: QueryResult> Query<R> {
    /// Shapes executor output into `R`.
    pub fn decode(&self, output: Output<'_>) -> Result<R, ResultError> {
        R::from_output(output)
    }
}

/// What an executor produced for a statement.
pub enum Output<'a> {
    Rows(&'a mut dyn RowCursor),
    Affected(u64),
}

/// A type a query can return.
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a query result type",
    note = "queries return `T`, `Option<T>` or `Vec<T>` (with `T: FromRow`), `u64` or `bool`"
)]
pub trait QueryResult: Sized {
    fn from_output(output: Output<'_>) -> Result<Self, ResultError>;
}

/// Result types for row-returning statements.
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a result type for a row-returning query",
    note = "selects return `T`, `Option<T>` or `Vec<T>` (with `T: FromRow`)"
)]
pub trait RowsResult: QueryResult {
    type Item: FromRow;
    const EXPECT: Expect;
}

/// Result types for row-modifying statements.
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a result type for insert, update or delete",
    note = "row-modifying queries return `u64` (affected rows) or `bool` (any row affected)"
)]
pub trait AffectedResult: QueryResult {}

fn rows(output: Output<'_>) -> Result<&mut dyn RowCursor, ResultError> {
    match output {
        Output::Rows(c) => Ok(c),
        Output::Affected(_) => Err(ResultError::ShapeMismatch),
    }
}

fn next<T: FromRow>(cursor: &mut dyn RowCursor) -> Result<Option<T>, ResultError> {
    match cursor.next_row() {
        None => Ok(None),
        Some(row) => T::from_row(row?).map(Some),
    }
}

impl<T: FromRow> QueryResult for T {
    fn from_output(output: Output<'_>) -> Result<Self, ResultError> {
        let cursor = rows(output)?;
        let first = next::<T>(cursor)?.ok_or(ResultError::NotFound)?;
        match cursor.next_row() {
            None => Ok(first),
            Some(_) => Err(ResultError::TooManyRows),
        }
    }
}
impl<T: FromRow> RowsResult for T {
    type Item = T;
    const EXPECT: Expect = Expect::ExactlyOne;
}

impl<T: FromRow> QueryResult for Option<T> {
    fn from_output(output: Output<'_>) -> Result<Self, ResultError> {
        let cursor = rows(output)?;
        let first = next::<T>(cursor)?;
        match cursor.next_row() {
            Some(_) if first.is_some() => Err(ResultError::TooManyRows),
            _ => Ok(first),
        }
    }
}
impl<T: FromRow> RowsResult for Option<T> {
    type Item = T;
    const EXPECT: Expect = Expect::AtMostOne;
}

impl<T: FromRow> QueryResult for Vec<T> {
    fn from_output(output: Output<'_>) -> Result<Self, ResultError> {
        let cursor = rows(output)?;
        let mut out = Vec::new();
        while let Some(item) = next::<T>(cursor)? {
            out.push(item);
        }
        Ok(out)
    }
}
impl<T: FromRow> RowsResult for Vec<T> {
    type Item = T;
    const EXPECT: Expect = Expect::Many;
}

impl QueryResult for u64 {
    fn from_output(output: Output<'_>) -> Result<Self, ResultError> {
        match output {
            Output::Affected(n) => Ok(n),
            Output::Rows(_) => Err(ResultError::ShapeMismatch),
        }
    }
}
impl AffectedResult for u64 {}

/// For DML: whether any row was affected. For `exists()`: the boolean row.
impl QueryResult for bool {
    fn from_output(output: Output<'_>) -> Result<Self, ResultError> {
        match output {
            Output::Affected(n) => Ok(n > 0),
            Output::Rows(cursor) => {
                let row = cursor.next_row().ok_or(ResultError::NotFound)??;
                Ok(bool::from_value(row.get(0, SqlType::Bool)?)?)
            }
        }
    }
}
impl AffectedResult for bool {}
