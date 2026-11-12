//! MySQL 8.0+ on [`sqlx::MySql`] (feature `mysql`).
//!
//! Values are stored as `rupa-driver-mysql` stores them: UUIDs and JSON as
//! text, `DateTime<Utc>` as a UTC `DATETIME(6)`, `bool` as `TINYINT(1)`.
//! sqlx connects with `CLIENT_FOUND_ROWS`, so `UPDATE` reports matched rows
//! as on Postgres, and sets the session time zone to UTC.
//!
//! The registry driver is `sqlx-mysql`, for `mysql:` URLs, with
//! [`MySqlOptions`](rupa_core::config::MySqlOptions): `statement_cache_size`, and `init` statements run on
//! every new connection. TLS is configured in the URL
//! (`?ssl-mode=REQUIRED`) and needs a `tls-*` feature.

#[cfg(feature = "pool")]
mod pool;

#[cfg(feature = "chrono")]
use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
use rupa_core::{RowError, SqlType, Value};
use sqlx::mysql::{MySqlArguments, MySqlRow};
use sqlx::{Arguments as _, Row as _, TypeInfo as _, ValueRef as _};

use crate::rows::{driver, int};
use crate::{Backend, BoxDynError, SqlxExecutor, SqlxTx};

#[cfg(feature = "dynamic")]
pub use pool::MySqlDriver;
#[cfg(feature = "pool")]
pub use pool::{MySqlManager, MySqlPool};

/// An executor on one `MySqlConnection`.
pub type MySqlExecutor = SqlxExecutor<sqlx::MySql>;
/// A transaction on a [`MySqlExecutor`].
pub type MySqlTx<'t> = SqlxTx<'t, sqlx::MySql>;

impl crate::sealed::Sealed for sqlx::MySql {}

impl Backend for sqlx::MySql {
    type Dialect = rupa_core::MySql;
    #[cfg(feature = "migrations")]
    type Migration = rupa_migrate::protocol::mysql::State;
    const DRIVER: &'static str = "sqlx-mysql";
    const SCHEMES: &'static [&'static str] = &["mysql"];

    fn bind(args: &mut MySqlArguments, value: &Value) -> Result<(), BoxDynError> {
        match value {
            Value::Null(ty) => null(args, *ty),
            Value::Bool(v) => args.add(*v),
            Value::I16(v) => args.add(*v),
            Value::I32(v) => args.add(*v),
            Value::I64(v) => args.add(*v),
            Value::F32(v) => args.add(*v),
            Value::F64(v) => args.add(*v),
            Value::Text(v) => args.add(v.as_str()),
            Value::Bytes(v) => args.add(v.as_slice()),
            #[cfg(feature = "uuid")]
            Value::Uuid(v) => args.add(v.hyphenated().to_string()),
            #[cfg(feature = "chrono")]
            Value::Date(v) => args.add(*v),
            #[cfg(feature = "chrono")]
            Value::Time(v) => args.add(*v),
            #[cfg(feature = "chrono")]
            Value::Timestamp(v) => args.add(*v),
            #[cfg(feature = "chrono")]
            Value::TimestampTz(v) => args.add(v.naive_utc()),
            Value::Json(v) => args.add(v.to_string()),
            other => Err(format!("unsupported parameter value {other:?}").into()),
        }
    }

    fn read(row: &MySqlRow, index: usize, want: SqlType) -> Result<Value, RowError> {
        let raw = row.try_get_raw(index).map_err(driver(index))?;
        if raw.is_null() {
            return Ok(Value::Null(want));
        }

        let column_type = raw.type_info().name().to_owned();
        let cell = cell(row, index, &column_type)?;
        match convert(&cell, want) {
            Some(Value::I64(n)) if want != SqlType::I64 => int(n, want, index),
            Some(value) => Ok(value),
            None => Err(RowError::Driver(format!(
                "column {index}: cannot read {column_type} value {cell:?} as {want:?}"
            ))),
        }
    }

    crate::sqlx_io!(sqlx::MySql);

    /// One statement at a time, as `rupa-driver-mysql` does.
    async fn script(conn: &mut sqlx::MySqlConnection, sql: &str) -> Result<(), sqlx::Error> {
        for statement in sql.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            sqlx::raw_sql(sqlx::AssertSqlSafe(statement))
                .execute(&mut *conn)
                .await?;
        }

        Ok(())
    }
}

/// A `NULL` of the MySQL type rupa stores `ty` as.
fn null(args: &mut MySqlArguments, ty: SqlType) -> Result<(), BoxDynError> {
    match ty {
        SqlType::Bool => args.add(None::<bool>),
        SqlType::I16 | SqlType::I32 | SqlType::I64 => args.add(None::<i64>),
        SqlType::F32 | SqlType::F64 => args.add(None::<f64>),
        SqlType::Text | SqlType::Uuid | SqlType::Json => args.add(None::<&str>),
        SqlType::Bytes => args.add(None::<&[u8]>),
        #[cfg(feature = "chrono")]
        SqlType::Date => args.add(None::<NaiveDate>),
        #[cfg(feature = "chrono")]
        SqlType::Time => args.add(None::<NaiveTime>),
        #[cfg(feature = "chrono")]
        SqlType::Timestamp | SqlType::TimestampTz => args.add(None::<NaiveDateTime>),
        #[cfg(not(feature = "chrono"))]
        other => Err(RowError::not_enabled(other).into()),
    }
}

/// A non-null MySQL value, decoded by its column type.
#[derive(Debug)]
enum Cell {
    Int(i64),
    UInt(u64),
    Float(f64),
    /// Text, binary strings, `DECIMAL` (as its digits) and JSON.
    Bytes(Vec<u8>),
    #[cfg(feature = "chrono")]
    Date(NaiveDate),
    #[cfg(feature = "chrono")]
    DateTime(NaiveDateTime),
    #[cfg(feature = "chrono")]
    Time(NaiveTime),
}

/// Decoding is unchecked: the column type picked the Rust type, and the
/// conversion to `want` happens in `convert`.
fn cell(row: &MySqlRow, index: usize, column_type: &str) -> Result<Cell, RowError> {
    macro_rules! get {
        ($t:ty) => {
            row.try_get_unchecked::<$t, _>(index)
                .map_err(driver(index))?
        };
    }
    Ok(match column_type {
        t if t.ends_with(" UNSIGNED") => Cell::UInt(get!(u64)),
        "BOOLEAN" | "TINYINT" | "SMALLINT" | "MEDIUMINT" | "INT" | "BIGINT" | "YEAR" => {
            Cell::Int(get!(i64))
        }
        "FLOAT" => Cell::Float(f64::from(get!(f32))),
        "DOUBLE" => Cell::Float(get!(f64)),
        #[cfg(feature = "chrono")]
        "DATE" => Cell::Date(get!(NaiveDate)),
        #[cfg(feature = "chrono")]
        "DATETIME" | "TIMESTAMP" => Cell::DateTime(get!(NaiveDateTime)),
        #[cfg(feature = "chrono")]
        "TIME" => Cell::Time(get!(NaiveTime)),
        _ => Cell::Bytes(get!(Vec<u8>)),
    })
}

/// `cell` as `want`, with the conversions `rupa-driver-mysql` makes.
/// Integers come back as `I64`, for the caller to narrow.
///
/// `DECIMAL` (e.g. `SUM` of integers) arrives as digits.
fn convert(cell: &Cell, want: SqlType) -> Option<Value> {
    let text = |b: &[u8]| String::from_utf8(b.to_vec()).ok();
    let integer = |cell: &Cell| -> Option<i64> {
        match cell {
            Cell::Int(n) => Some(*n),
            Cell::UInt(n) => i64::try_from(*n).ok(),
            Cell::Bytes(b) => std::str::from_utf8(b).ok()?.parse().ok(),
            _ => None,
        }
    };
    let float = |cell: &Cell| -> Option<f64> {
        match cell {
            Cell::Float(f) => Some(*f),
            Cell::Int(n) => Some(*n as f64),
            Cell::Bytes(b) => std::str::from_utf8(b).ok()?.parse().ok(),
            _ => None,
        }
    };
    Some(match want {
        SqlType::Bool => Value::Bool(integer(cell)? != 0),
        SqlType::I16 | SqlType::I32 | SqlType::I64 => Value::I64(integer(cell)?),
        SqlType::F32 => Value::F32(float(cell)? as f32),
        SqlType::F64 => Value::F64(float(cell)?),
        SqlType::Text => match cell {
            Cell::Bytes(b) => Value::Text(text(b)?),
            _ => return None,
        },
        SqlType::Bytes => match cell {
            Cell::Bytes(b) => Value::Bytes(b.clone()),
            _ => return None,
        },
        #[cfg(feature = "uuid")]
        SqlType::Uuid => match cell {
            Cell::Bytes(b) if b.len() == 16 => Value::Uuid(uuid::Uuid::from_slice(b).ok()?),
            Cell::Bytes(b) => Value::Uuid(text(b)?.parse().ok()?),
            _ => return None,
        },
        #[cfg(feature = "chrono")]
        SqlType::Date => match cell {
            Cell::Date(d) => Value::Date(*d),
            Cell::DateTime(t) => Value::Date(t.date()),
            _ => return None,
        },
        #[cfg(feature = "chrono")]
        SqlType::Time => match cell {
            Cell::Time(t) => Value::Time(*t),
            _ => return None,
        },
        #[cfg(feature = "chrono")]
        SqlType::Timestamp => match cell {
            Cell::DateTime(t) => Value::Timestamp(*t),
            _ => return None,
        },
        #[cfg(feature = "chrono")]
        SqlType::TimestampTz => match cell {
            Cell::DateTime(t) => Value::TimestampTz(t.and_utc()),
            _ => return None,
        },
        SqlType::Json => match cell {
            Cell::Bytes(b) => Value::Json(serde_json::from_slice(b).ok()?),
            _ => return None,
        },
        #[cfg(not(all(feature = "chrono", feature = "uuid")))]
        _ => return None,
    })
}
