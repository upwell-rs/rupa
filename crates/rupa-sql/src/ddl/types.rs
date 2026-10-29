//! Per-dialect column types and default literals.
//!
//! Types match what each driver binds and decodes for the corresponding
//! `SqlType`, and the conformance schemas: MySQL stores UUIDs as `CHAR(36)`
//! and both timestamp kinds as `DATETIME(6)` (UTC); SQLite stores
//! everything without a native type as `TEXT`. Unbounded text and bytes use
//! MySQL's `LONGTEXT`/`LONGBLOB`, matching PostgreSQL's capacity.

use std::fmt::Write as _;

use rupa_core::schema::{ColumnDef, ColumnDefault, ColumnType};
use rupa_core::{DialectId, Value};

use super::SchemaError;

/// The type name of `column` on `dialect`.
pub(super) fn type_name(column: &ColumnDef, dialect: DialectId) -> Result<String, SchemaError> {
    let invalid = || SchemaError::InvalidType {
        column: column.name,
        ty: column.ty,
    };
    let name = match (column.ty, dialect) {
        (ColumnType::Custom(custom), _) => {
            let name = match dialect {
                DialectId::Postgres => custom.postgres,
                DialectId::MySql => custom.mysql,
                DialectId::Sqlite => custom.sqlite,
                _ => None,
            };

            return name.map(str::to_owned).ok_or(SchemaError::CustomType {
                column: column.name,
                dialect,
            });
        }
        (ColumnType::VarChar(0), _) => return Err(invalid()),
        (ColumnType::Decimal { precision, scale }, _)
            if precision == 0 || precision > 65 || scale > precision =>
        {
            return Err(invalid());
        }
        (ColumnType::Decimal { precision, scale }, DialectId::Postgres) => {
            return Ok(format!("NUMERIC({precision}, {scale})"));
        }
        (ColumnType::Decimal { precision, scale }, DialectId::MySql) => {
            return Ok(format!("DECIMAL({precision}, {scale})"));
        }
        (ColumnType::VarChar(n), DialectId::Postgres | DialectId::MySql) => {
            return Ok(format!("VARCHAR({n})"));
        }
        (ty, DialectId::Postgres) => postgres(ty),
        (ty, DialectId::MySql) => mysql(ty),
        (ty, _) => sqlite(ty),
    };

    Ok(name.to_owned())
}

fn postgres(ty: ColumnType) -> &'static str {
    match ty {
        ColumnType::Bool => "BOOLEAN",
        ColumnType::SmallInt => "SMALLINT",
        ColumnType::Int => "INTEGER",
        ColumnType::BigInt => "BIGINT",
        ColumnType::Real => "REAL",
        ColumnType::Double => "DOUBLE PRECISION",
        ColumnType::Text => "TEXT",
        ColumnType::Bytes => "BYTEA",
        ColumnType::Uuid => "UUID",
        ColumnType::Date => "DATE",
        ColumnType::Time => "TIME",
        ColumnType::Timestamp => "TIMESTAMP",
        ColumnType::TimestampTz => "TIMESTAMPTZ",
        ColumnType::Json => "JSONB",
        ColumnType::Decimal { .. } | ColumnType::VarChar(_) | ColumnType::Custom(_) => {
            unreachable!("handled by type_name")
        }
    }
}

fn mysql(ty: ColumnType) -> &'static str {
    match ty {
        ColumnType::Bool => "BOOLEAN",
        ColumnType::SmallInt => "SMALLINT",
        ColumnType::Int => "INT",
        ColumnType::BigInt => "BIGINT",
        ColumnType::Real => "FLOAT",
        ColumnType::Double => "DOUBLE",
        ColumnType::Text => "LONGTEXT",
        ColumnType::Bytes => "LONGBLOB",
        ColumnType::Uuid => "CHAR(36)",
        ColumnType::Date => "DATE",
        ColumnType::Time => "TIME(6)",
        ColumnType::Timestamp | ColumnType::TimestampTz => "DATETIME(6)",
        ColumnType::Json => "JSON",
        ColumnType::Decimal { .. } | ColumnType::VarChar(_) | ColumnType::Custom(_) => {
            unreachable!("handled by type_name")
        }
    }
}

fn sqlite(ty: ColumnType) -> &'static str {
    match ty {
        ColumnType::Bool | ColumnType::SmallInt | ColumnType::Int | ColumnType::BigInt => "INTEGER",
        ColumnType::Real | ColumnType::Double => "REAL",
        ColumnType::Decimal { .. } => "NUMERIC",
        ColumnType::Bytes => "BLOB",
        ColumnType::Text
        | ColumnType::VarChar(_)
        | ColumnType::Uuid
        | ColumnType::Date
        | ColumnType::Time
        | ColumnType::Timestamp
        | ColumnType::TimestampTz
        | ColumnType::Json => "TEXT",
        ColumnType::Custom(_) => unreachable!("handled by type_name"),
    }
}

/// Whether `ty` is an integer type, as identity columns require.
pub(super) fn is_integer(ty: ColumnType) -> bool {
    matches!(
        ty,
        ColumnType::SmallInt | ColumnType::Int | ColumnType::BigInt
    )
}

/// MySQL cannot key or index these without a prefix length, and needs an
/// expression (parenthesized) default for them.
pub(super) fn mysql_unbounded(ty: ColumnType) -> bool {
    matches!(ty, ColumnType::Text | ColumnType::Bytes | ColumnType::Json)
}

/// The `DEFAULT` clause value of `column`.
///
/// Expressions are parenthesized, the form every dialect accepts for any
/// type. Text is quoted with doubled quotes. A backslash is refused on MySQL,
/// where its meaning depends on the server's SQL mode, and written as an
/// `E'..'` escape string on PostgreSQL, which means the same whatever
/// `standard_conforming_strings` is.
pub(super) fn default(
    column: &ColumnDef,
    default: &ColumnDefault,
    dialect: DialectId,
) -> Result<String, SchemaError> {
    let refused = || SchemaError::Default {
        column: column.name,
        dialect,
    };
    let mut out = String::new();
    let literal = match default {
        ColumnDefault::Sql(sql) => return Ok(format!("({sql})")),
        ColumnDefault::Value(value) => value,
    };

    match literal {
        Value::Bool(b) if dialect == DialectId::Sqlite => out.push(if *b { '1' } else { '0' }),
        Value::Bool(b) => out.push_str(if *b { "TRUE" } else { "FALSE" }),
        Value::I16(n) => _ = write!(out, "{n}"),
        Value::I32(n) => _ = write!(out, "{n}"),
        Value::I64(n) => _ = write!(out, "{n}"),
        Value::F32(n) if n.is_finite() => _ = write!(out, "{n}"),
        Value::F64(n) if n.is_finite() => _ = write!(out, "{n}"),
        Value::Text(text) => quoted(text, dialect, &mut out).ok_or_else(refused)?,
        Value::Json(json) => quoted(&json.to_string(), dialect, &mut out).ok_or_else(refused)?,
        _ => return Err(refused()),
    }

    if dialect == DialectId::MySql && mysql_unbounded(column.ty) {
        return Ok(format!("({out})"));
    }

    Ok(out)
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
