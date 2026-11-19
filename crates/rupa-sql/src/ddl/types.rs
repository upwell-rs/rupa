//! Per-dialect column types and default literals.
//!
//! Types match what each driver binds and decodes for the corresponding
//! `SqlType`, and the conformance schemas: MySQL stores UUIDs as `CHAR(36)`
//! and both timestamp kinds as `DATETIME(6)` (UTC); SQLite stores
//! everything without a native type as `TEXT`. Unbounded text and bytes use
//! MySQL's `LONGTEXT`/`LONGBLOB`, matching PostgreSQL's capacity.

use rupa_core::DialectId;
use rupa_core::schema::{ColumnDef, ColumnDefault, ColumnSpec, ColumnType, TypeSupport};

use super::{SchemaError, literal};

/// The type name of `column` on `dialect`.
pub(super) fn type_name(column: &ColumnDef, dialect: DialectId) -> Result<String, SchemaError> {
    let invalid = || SchemaError::InvalidType {
        column: column.name,
        ty: column.ty,
    };
    let name = match (column.ty, dialect) {
        (ColumnType::Kind(spec), _) => return kind_name(column, *spec, dialect),
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

/// A kind's type on `dialect`: `name(arg, ..)`, or `name` without arguments.
fn kind_name(
    column: &ColumnDef,
    spec: ColumnSpec,
    dialect: DialectId,
) -> Result<String, SchemaError> {
    let (name, args) = match spec.support(dialect) {
        TypeSupport::Named { name, args } => (name, args),
        TypeSupport::Unsupported(reason) => {
            return Err(SchemaError::UnsupportedType {
                column: column.name,
                kind: spec.name,
                dialect,
                reason,
            });
        }
    };
    let args = args.iter().map(u32::to_string).collect::<Vec<_>>();

    if args.is_empty() {
        return Ok(name.to_owned());
    }

    Ok(format!("{name}({})", args.join(", ")))
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
        ColumnType::Decimal { .. } | ColumnType::VarChar(_) | ColumnType::Kind(_) => {
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
        ColumnType::Decimal { .. } | ColumnType::VarChar(_) | ColumnType::Kind(_) => {
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
        ColumnType::Kind(_) => unreachable!("handled by type_name"),
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
    match ty {
        ColumnType::Kind(spec) => !spec.bounded,
        ty => matches!(ty, ColumnType::Text | ColumnType::Bytes | ColumnType::Json),
    }
}

/// The `DEFAULT` clause value of `column`.
///
/// Expressions are parenthesized, the form every dialect accepts for any
/// type. Values are written as checked [`literal`]s.
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
    let value = match default {
        ColumnDefault::Sql(sql) => return Ok(format!("({sql})")),
        ColumnDefault::Value(value) => value,
    };

    literal::write(value, dialect, &mut out).ok_or_else(refused)?;

    if dialect == DialectId::MySql && mysql_unbounded(column.ty) {
        return Ok(format!("({out})"));
    }

    Ok(out)
}
