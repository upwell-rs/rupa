//! MySQL 8.0+.
//!
//! Where MySQL's behaviour differs from Postgres', the SQL is written to
//! reproduce Postgres' (the reference semantics every backend matches):
//!
//! - `ORDER BY`: NULLs sort first ascending in MySQL. Keys are emulated as
//!   `(x IS NULL) ASC, x ASC` (and `DESC, DESC`) for `NULLS LAST`/`FIRST`.
//! - `OFFSET` needs a `LIMIT`: an unbounded one is written.
//! - `->>` turns JSON `null` into the string `'null'`: text paths are written
//!   with `JSON_TYPE` so a JSON `null` (like a missing key) is SQL `NULL`.
//! - There is no `CAST(.. AS BOOLEAN)`: a JSON value is true when its text
//!   is `true` or `1`.
//!
//! Not reproducible in SQL: text comparison and `LIKE` follow the column's
//! collation. Use a binary or case-sensitive collation (`utf8mb4_bin`) for
//! Postgres-like results. Drivers must report *matched* rows for `UPDATE`
//! (`CLIENT_FOUND_ROWS`), as Postgres does.

use std::fmt::Write as _;

use rupa_core::SqlType;
use rupa_core::ir::{PathSeg, TxStatement};

use crate::RenderError;
use crate::writer::{NullsOrdering, Syntax, common_tx, isolation_sql};

pub(crate) struct MySql;

/// `'$."a"[0]."b"'`. Keys are static text; ones needing backslash escapes
/// inside a JSON path are refused, since how MySQL reads backslashes in
/// string literals depends on the server's SQL mode.
pub(crate) fn json_path_literal(
    path: &[PathSeg],
    refuse_quotes: bool,
) -> Result<String, RenderError> {
    let mut p = String::from("$");
    for seg in path {
        match seg {
            PathSeg::Key(k) => {
                if k.chars()
                    .any(|c| c == '\0' || c == '\\' || (refuse_quotes && c == '"'))
                {
                    return Err(RenderError::InvalidJsonKey(k));
                }
                let _ = write!(p, ".\"{k}\"");
            }
            PathSeg::Index(n) => {
                let _ = write!(p, "[{n}]");
            }
        }
    }
    Ok(format!("'{}'", p.replace('\'', "''")))
}

impl Syntax for MySql {
    fn placeholder(&self, _n: usize, out: &mut String) {
        out.push('?');
    }

    fn quote_ident(&self, ident: &str, out: &mut String) {
        out.push('`');
        for c in ident.chars() {
            if c == '`' {
                out.push('`');
            }
            out.push(c);
        }
        out.push('`');
    }

    fn cast(&self, inner: &str, ty: SqlType) -> Result<String, RenderError> {
        let target = match ty {
            SqlType::Bool => return Ok(format!("({inner} IN ('true', '1'))")),
            SqlType::I16 | SqlType::I32 | SqlType::I64 => "SIGNED",
            SqlType::F32 | SqlType::F64 => "DOUBLE",
            SqlType::Text => "CHAR",
            SqlType::Bytes => "BINARY",
            SqlType::Uuid => "CHAR(36)",
            SqlType::Date => "DATE",
            SqlType::Time => "TIME(6)",
            SqlType::Timestamp | SqlType::TimestampTz => "DATETIME(6)",
            SqlType::Json => "JSON",
        };
        Ok(format!("CAST({inner} AS {target})"))
    }

    fn json_path(
        &self,
        column: &str,
        path: &[PathSeg],
        as_text: bool,
        out: &mut String,
    ) -> Result<(), RenderError> {
        let p = json_path_literal(path, true)?;
        let extract = format!("JSON_EXTRACT({column}, {p})");
        if as_text {
            let _ = write!(
                out,
                "(CASE WHEN JSON_TYPE({extract}) = 'NULL' THEN NULL ELSE JSON_UNQUOTE({extract}) END)"
            );
        } else {
            out.push_str(&extract);
        }
        Ok(())
    }

    fn nulls_ordering(&self) -> NullsOrdering {
        NullsOrdering::Emulate
    }

    fn unbounded_limit(&self) -> Option<&'static str> {
        Some("18446744073709551615")
    }

    fn tx(&self, statement: &TxStatement) -> Result<String, RenderError> {
        if let Some(sql) = common_tx(statement) {
            return Ok(sql);
        }
        let TxStatement::Begin(options) = statement else {
            unreachable!()
        };
        let mut sql = String::new();
        if let Some(level) = options.isolation {
            // Applies to the next transaction only.
            let _ = write!(
                sql,
                "SET TRANSACTION ISOLATION LEVEL {}; ",
                isolation_sql(level)
            );
        }
        sql.push_str("START TRANSACTION");
        if options.read_only {
            sql.push_str(" READ ONLY");
        }
        Ok(sql)
    }
}
