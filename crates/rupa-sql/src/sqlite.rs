//! SQLite 3.38+ (`->`/`->>` JSON operators; `RETURNING` since 3.35).
//!
//! Where SQLite's behaviour differs from Postgres', the SQL reproduces
//! Postgres' (the reference semantics every backend matches):
//!
//! - `ORDER BY` keys get `NULLS LAST` (ascending) / `NULLS FIRST` (descending).
//! - `OFFSET` needs a `LIMIT`: `LIMIT -1` is written.
//! - `LIKE` has no default escape character: `ESCAPE '\'` is written.
//!
//! Not expressible here:
//! - `LIKE` is case-insensitive by default; drivers enable
//!   `PRAGMA case_sensitive_like`.
//! - Read-only transactions are refused (`ReadOnlyTransactions` capability)
//!   rather than silently ignored.
//! - Every isolation level is accepted: SQLite transactions are serializable,
//!   which satisfies any requested level.

use rupa_core::ir::{PathSeg, TxStatement};
use rupa_core::{Capability, SqlType};

use crate::RenderError;
use crate::mysql::json_path_literal;
use crate::writer::{NullsOrdering, Syntax, common_tx};

pub(crate) struct Sqlite;

impl Syntax for Sqlite {
    fn placeholder(&self, _n: usize, out: &mut String) {
        out.push('?');
    }

    fn cast(&self, inner: &str, ty: SqlType) -> Result<String, RenderError> {
        let target = match ty {
            SqlType::Bool | SqlType::I16 | SqlType::I32 | SqlType::I64 => "INTEGER",
            SqlType::F32 | SqlType::F64 => "REAL",
            SqlType::Bytes => "BLOB",
            SqlType::Text
            | SqlType::Uuid
            | SqlType::Date
            | SqlType::Time
            | SqlType::Timestamp
            | SqlType::TimestampTz
            | SqlType::Json => "TEXT",
        };
        Ok(format!("CAST({inner} AS {target})"))
    }

    /// `"col" ->> '$."a"[0]'`: SQLite's `->>` already yields SQL `NULL` for a
    /// JSON `null`.
    fn json_path(
        &self,
        column: &str,
        path: &[PathSeg],
        as_text: bool,
        out: &mut String,
    ) -> Result<(), RenderError> {
        out.push_str(column);
        out.push_str(if as_text { " ->> " } else { " -> " });
        out.push_str(&json_path_literal(path, true)?);
        Ok(())
    }

    fn nulls_ordering(&self) -> NullsOrdering {
        NullsOrdering::Keyword
    }

    fn unbounded_limit(&self) -> Option<&'static str> {
        Some("-1")
    }

    fn like_escape(&self) -> &'static str {
        " ESCAPE '\\'"
    }

    fn tx(&self, statement: &TxStatement) -> Result<String, RenderError> {
        if let Some(sql) = common_tx(statement) {
            return Ok(sql);
        }
        let TxStatement::Begin(options) = statement else {
            unreachable!()
        };
        if options.read_only {
            return Err(RenderError::UnsupportedCapability(
                Capability::ReadOnlyTransactions,
            ));
        }
        Ok("BEGIN".into())
    }
}
