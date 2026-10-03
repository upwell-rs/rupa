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

use std::fmt;

use rupa_core::ir::{Statement, TxStatement};
use rupa_core::{Dialect, DialectId, DslError, Query, Value};

mod mysql;
mod postgres;
mod sqlite;
mod writer;

/// SQL text and its parameters, in placeholder order.
#[derive(Debug, Clone, PartialEq)]
pub struct Rendered {
    pub sql: String,
    pub params: Vec<Value>,
}

#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum RenderError {
    /// No renderer exists yet for this dialect.
    UnsupportedDialect(DialectId),
    /// A DSL function failed to lower (including "unsupported on this dialect").
    Dsl(DslError),
    /// The statement needs a capability the dialect lacks (e.g. `RETURNING`).
    UnsupportedCapability(rupa_core::Capability),
    /// A security-context key that is not a valid setting name.
    InvalidSecurityKey(String),
    /// An `INSERT` with no rows.
    EmptyInsert,
    /// An `UPDATE` with no assignments.
    EmptyUpdate,
    /// An operator or function name outside the allowed character set.
    InvalidToken(&'static str),
    /// A JSON path key the dialect cannot express safely.
    InvalidJsonKey(&'static str),
    /// DSL lowering nested deeper than the renderer allows (likely a lowering cycle).
    TooDeep,
}

impl From<DslError> for RenderError {
    fn from(e: DslError) -> Self {
        RenderError::Dsl(e)
    }
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenderError::UnsupportedDialect(id) => write!(f, "no SQL renderer for dialect {id:?}"),
            RenderError::Dsl(e) => e.fmt(f),
            RenderError::UnsupportedCapability(c) => write!(f, "dialect does not support {c:?}"),
            RenderError::InvalidSecurityKey(k) => write!(f, "invalid security context key {k:?}"),
            RenderError::EmptyInsert => f.write_str("INSERT without rows"),
            RenderError::EmptyUpdate => f.write_str("UPDATE without assignments"),
            RenderError::InvalidToken(t) => write!(f, "invalid operator or function name `{t}`"),
            RenderError::InvalidJsonKey(k) => write!(f, "invalid JSON path key {k:?}"),
            RenderError::TooDeep => f.write_str("expression nesting too deep"),
        }
    }
}

impl std::error::Error for RenderError {}

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
