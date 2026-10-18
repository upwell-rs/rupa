//! Renders [`rupa_core::ir`] statements to SQL text plus bound parameters.
//!
//! Values are never interpolated: every [`Value`] becomes a placeholder and an
//! entry in [`Rendered::params`]. The only text the renderer writes besides
//! keywords is `&'static str` from the IR (identifiers, operators, function
//! names, JSON keys, raw fragments). Identifiers are quoted, and operator and
//! function names are checked against a conservative character set.
//!
//! DSL calls are lowered here, with the target dialect, so a statement stays
//! dialect-free until this point.
//!
//! [`render`] renders every time. Drivers call [`render_cached`] instead,
//! which reuses the SQL of earlier statements with the same *shape* (never
//! the same values; see [`cache`]).

#![warn(missing_docs)]

use rupa_core::ir::{SourceId, Statement, TxStatement};
use rupa_core::{Dialect, DialectId, DslError, Query, Value};

pub mod cache;
mod driver_error;
mod mysql;
mod postgres;
mod sqlite;
mod writer;

pub use cache::{CacheStats, CachedSql, RenderCache, render_cache, render_cached};

/// SQL text and its parameters, in placeholder order.
#[derive(Debug, Clone, PartialEq)]
pub struct Rendered {
    /// The SQL text, with the dialect's placeholders (`$1`, `?`).
    pub sql: String,
    /// The values to bind, in placeholder order.
    pub params: Vec<Value>,
}

/// Why a statement could not be rendered.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum RenderError {
    /// No renderer exists yet for this dialect.
    #[error("no SQL renderer for dialect {0:?}")]
    UnsupportedDialect(DialectId),
    /// A DSL function failed to lower (including "unsupported on this dialect").
    #[error(transparent)]
    Dsl(#[from] DslError),
    /// The statement needs a capability the dialect lacks (e.g. `RETURNING`).
    #[error("dialect does not support {0:?}")]
    UnsupportedCapability(rupa_core::Capability),
    /// A security-context key that is not a valid setting name.
    #[error("invalid security context key {0:?}")]
    InvalidSecurityKey(String),
    /// An `INSERT` with no rows.
    #[error("INSERT without rows")]
    EmptyInsert,
    /// An `UPDATE` with no assignments.
    #[error("UPDATE without assignments")]
    EmptyUpdate,
    /// An operator or function name outside the allowed character set.
    #[error("invalid operator or function name `{0}`")]
    InvalidToken(&'static str),
    /// A JSON path key the dialect cannot express safely.
    #[error("invalid JSON path key {0:?}")]
    InvalidJsonKey(&'static str),
    /// DSL lowering nested deeper than the renderer allows (likely a lowering cycle).
    #[error("expression nesting too deep")]
    TooDeep,
    /// A column names a row source the statement doesn't have (e.g. a joined
    /// source in an `UPDATE` filter).
    #[error("column refers to row source {} the statement doesn't have", .0.0)]
    UnknownSource(SourceId),
    /// More joined sources than a `SourceId` can number.
    #[error("too many joined sources")]
    TooManySources,
}

fn syntax_for(dialect: &dyn Dialect) -> Result<&'static dyn writer::Syntax, RenderError> {
    Ok(match dialect.id() {
        DialectId::Postgres => &postgres::Postgres,
        DialectId::MySql => &mysql::MySql,
        DialectId::Sqlite => &sqlite::Sqlite,
        other => return Err(RenderError::UnsupportedDialect(other)),
    })
}

/// Renders a statement for `dialect`.
pub fn render(statement: &Statement, dialect: &dyn Dialect) -> Result<Rendered, RenderError> {
    writer::Writer::new(dialect, syntax_for(dialect)?).statement(statement)
}

/// Renders a query's statement for `dialect`.
pub fn render_query<R>(query: &Query<R>, dialect: &dyn Dialect) -> Result<Rendered, RenderError> {
    render(query.statement(), dialect)
}

/// Renders a transaction control statement for `dialect`. The result may hold
/// several statements; drivers send it with their simple-query path.
pub fn render_tx(statement: &TxStatement, dialect: &dyn Dialect) -> Result<String, RenderError> {
    syntax_for(dialect)?.tx(statement)
}

/// Paths used by [`driver_error!`]'s output. Not public API.
#[doc(hidden)]
pub mod __private {
    pub use rupa_core;
}
