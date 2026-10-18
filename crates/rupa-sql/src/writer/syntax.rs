//! [`Syntax`]: the parts of SQL that differ between dialects.

use rupa_core::SqlType;
use rupa_core::ir::{PathSeg, TxStatement};

use crate::RenderError;

/// How a dialect gets Postgres' default NULL ordering (`NULLS LAST`
/// ascending, `NULLS FIRST` descending), which every backend must match.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum NullsOrdering {
    /// The dialect's default already matches.
    Default,
    /// `NULLS LAST` / `NULLS FIRST` keywords.
    Keyword,
    /// No keyword: sort by `(x IS NULL)` first.
    Emulate,
}

pub(crate) trait Syntax {
    /// Writes the placeholder for 1-based parameter `n`.
    fn placeholder(&self, n: usize, out: &mut String);

    fn quote_ident(&self, ident: &str, out: &mut String) {
        out.push('"');
        for c in ident.chars() {
            if c == '"' {
                out.push('"');
            }

            out.push(c);
        }

        out.push('"');
    }

    /// `inner` (already rendered) converted to `ty`.
    fn cast(&self, inner: &str, ty: SqlType) -> Result<String, RenderError>;

    /// Writes a path into the already-rendered JSON column `column`.
    /// `as_text` must give SQL `NULL` for a missing key or a JSON `null`.
    fn json_path(
        &self,
        column: &str,
        path: &[PathSeg],
        as_text: bool,
        out: &mut String,
    ) -> Result<(), RenderError>;

    fn nulls_ordering(&self) -> NullsOrdering {
        NullsOrdering::Default
    }

    /// The `LIMIT` to write when only `OFFSET` is given, if the dialect
    /// requires one.
    fn unbounded_limit(&self) -> Option<&'static str> {
        None
    }

    /// Written after a `LIKE` pattern, making `\` the escape character.
    fn like_escape(&self) -> &'static str {
        ""
    }

    /// Transaction control; may hold several statements.
    fn tx(&self, statement: &TxStatement) -> Result<String, RenderError>;
}

/// Savepoint name for nesting depth `depth`.
pub(crate) fn savepoint(depth: u32) -> String {
    format!("rupa_sp_{depth}")
}

/// The savepoint and commit/rollback statements, identical across dialects.
pub(crate) fn common_tx(statement: &TxStatement) -> Option<String> {
    Some(match statement {
        TxStatement::Begin(_) => return None,
        TxStatement::Commit => "COMMIT".into(),
        TxStatement::Rollback => "ROLLBACK".into(),
        TxStatement::Savepoint(d) => format!("SAVEPOINT {}", savepoint(*d)),
        TxStatement::ReleaseSavepoint(d) => format!("RELEASE SAVEPOINT {}", savepoint(*d)),
        TxStatement::RollbackToSavepoint(d) => {
            let sp = savepoint(*d);
            format!("ROLLBACK TO SAVEPOINT {sp}; RELEASE SAVEPOINT {sp}")
        }
    })
}

pub(crate) fn isolation_sql(level: rupa_core::ir::IsolationLevel) -> &'static str {
    use rupa_core::ir::IsolationLevel::*;
    match level {
        ReadUncommitted => "READ UNCOMMITTED",
        ReadCommitted => "READ COMMITTED",
        RepeatableRead => "REPEATABLE READ",
        Serializable => "SERIALIZABLE",
    }
}
