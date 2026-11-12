//! Checked literals: the only way data is written into DDL.

use std::fmt::Write as _;

use rupa_core::{DialectId, Value};

/// Writes `value` as a literal of `dialect` into `out`, or returns `None`
/// when it has no safe literal form there.
///
/// Booleans, integers, finite floats, text and JSON are supported. Text is
/// quoted with doubled quotes. A backslash is refused on MySQL, where its
/// meaning depends on the server's SQL mode, and written as an `E'..'` escape
/// string on PostgreSQL, which means the same whatever
/// `standard_conforming_strings` is.
pub(crate) fn write(value: &Value, dialect: DialectId, out: &mut String) -> Option<()> {
    match value {
        Value::Bool(b) if dialect == DialectId::Sqlite => out.push(if *b { '1' } else { '0' }),
        Value::Bool(b) => out.push_str(if *b { "TRUE" } else { "FALSE" }),
        Value::I16(n) => _ = write!(out, "{n}"),
        Value::I32(n) => _ = write!(out, "{n}"),
        Value::I64(n) => _ = write!(out, "{n}"),
        Value::F32(n) if n.is_finite() => _ = write!(out, "{n}"),
        Value::F64(n) if n.is_finite() => _ = write!(out, "{n}"),
        Value::Text(text) => quoted(text, dialect, out)?,
        Value::Json(json) => quoted(&json.to_string(), dialect, out)?,
        _ => return None,
    }

    Some(())
}

fn quoted(text: &str, dialect: DialectId, out: &mut String) -> Option<()> {
    if text.contains('\0') || (dialect == DialectId::MySql && text.contains('\\')) {
        return None;
    }

    let escape = dialect == DialectId::Postgres && text.contains('\\');

    if escape {
        out.push('E');
    }

    out.push('\'');

    for c in text.chars() {
        match c {
            '\'' => out.push_str("''"),
            '\\' if escape => out.push_str("\\\\"),
            c => out.push(c),
        }
    }

    out.push('\'');

    Some(())
}
