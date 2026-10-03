use std::fmt::Write as _;

use rupa_core::SqlType;
use rupa_core::ir::PathSeg;

use crate::RenderError;
use crate::writer::{Syntax, common_tx, isolation_sql};
use rupa_core::ir::TxStatement;

pub(crate) struct Postgres;

fn cast_type(ty: SqlType) -> &'static str {
    match ty {
        SqlType::Bool => "boolean",
        SqlType::I16 => "smallint",
        SqlType::I32 => "integer",
        SqlType::I64 => "bigint",
        SqlType::F32 => "real",
        SqlType::F64 => "double precision",
        SqlType::Text => "text",
        SqlType::Bytes => "bytea",
        SqlType::Uuid => "uuid",
        SqlType::Date => "date",
        SqlType::Time => "time",
        SqlType::Timestamp => "timestamp",
        SqlType::TimestampTz => "timestamptz",
        SqlType::Json => "jsonb",
    }
}

impl Syntax for Postgres {
    fn placeholder(&self, n: usize, out: &mut String) {
        let _ = write!(out, "${n}");
    }

    fn cast(&self, inner: &str, ty: SqlType) -> Result<String, RenderError> {
        Ok(format!("CAST({inner} AS {})", cast_type(ty)))
    }

    fn tx(&self, statement: &TxStatement) -> Result<String, RenderError> {
        if let Some(sql) = common_tx(statement) {
            return Ok(sql);
        }
        let TxStatement::Begin(options) = statement else {
            unreachable!()
        };
        let mut sql = String::from("BEGIN");
        if let Some(level) = options.isolation {
            sql.push_str(" ISOLATION LEVEL ");
            sql.push_str(isolation_sql(level));
        }
        if options.read_only {
            sql.push_str(" READ ONLY");
        }
        Ok(sql)
    }

    /// `"col" -> 'a' -> 0 ->> 'b'`. Keys are author-written static text,
    /// rendered as string literals rather than bound, so that expression
    /// indexes on `col ->> 'key'` still match.
    fn json_path(
        &self,
        column: &str,
        path: &[PathSeg],
        as_text: bool,
        out: &mut String,
    ) -> Result<(), RenderError> {
        out.push_str(column);
        for (i, seg) in path.iter().enumerate() {
            let last = i + 1 == path.len();
            out.push_str(if last && as_text { " ->> " } else { " -> " });
            match seg {
                PathSeg::Key(k) => string_literal(k, out)?,
                PathSeg::Index(n) => {
                    let _ = write!(out, "{n}");
                }
            }
        }
        Ok(())
    }
}

/// A string literal that means the same regardless of
/// `standard_conforming_strings`: plain `'...'` when the text has no
/// backslash, otherwise an `E'...'` escape string.
fn string_literal(s: &'static str, out: &mut String) -> Result<(), RenderError> {
    if s.contains('\0') {
        return Err(RenderError::InvalidJsonKey(s));
    }
    let escape = s.contains('\\');
    if escape {
        out.push('E');
    }
    out.push('\'');
    for c in s.chars() {
        match c {
            '\'' => out.push_str("''"),
            '\\' => out.push_str("\\\\"),
            c => out.push(c),
        }
    }
    out.push('\'');
    Ok(())
}
