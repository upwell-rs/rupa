//! SQLite 3.38+ on [`sqlx::Sqlite`] (feature `sqlite`; SQLite is compiled
//! in).
//!
//! Values are stored as `rupa-driver-rusqlite` stores them, so a database
//! file can be used by either driver: integers and booleans as `INTEGER`,
//! floats as `REAL`, UUIDs as hyphenated text, dates and times as ISO-8601
//! text with microseconds (`DateTime<Utc>` with a `Z`), JSON as text.
//!
//! The registry driver is `sqlx-sqlite`, for `sqlite:` URLs (read as
//! [`SqliteTarget`](rupa_core::config::SqliteTarget) reads them; an in-memory database gets a single
//! connection), with [`SqliteOptions`](rupa_core::config::SqliteOptions).
//!
//! sqlx runs each SQLite connection on its own worker thread, so statements
//! don't block the async runtime.

#[cfg(feature = "pool")]
mod pool;

#[cfg(feature = "chrono")]
use rupa_core::chrono_text;
use rupa_core::{RowError, SqlType, Value};
use sqlx::sqlite::{SqliteArguments, SqliteRow};
use sqlx::{Arguments as _, Row as _, TypeInfo as _, ValueRef as _};

use crate::rows::{driver, int};
use crate::{Backend, BoxDynError, SqlxExecutor, SqlxTx};

#[cfg(feature = "dynamic")]
pub use pool::SqliteDriver;
#[cfg(feature = "pool")]
pub use pool::{SqliteManager, SqlitePool};

/// An executor on one `SqliteConnection`.
pub type SqliteExecutor = SqlxExecutor<sqlx::Sqlite>;
/// A transaction on a [`SqliteExecutor`].
pub type SqliteTx<'t> = SqlxTx<'t, sqlx::Sqlite>;

impl crate::sealed::Sealed for sqlx::Sqlite {}

impl Backend for sqlx::Sqlite {
    type Dialect = rupa_core::Sqlite;
    const DRIVER: &'static str = "sqlx-sqlite";
    const SCHEMES: &'static [&'static str] = &["sqlite"];

    fn bind(args: &mut SqliteArguments, value: &Value) -> Result<(), BoxDynError> {
        match value {
            Value::Null(_) => args.add(None::<i64>),
            Value::Bool(v) => args.add(i64::from(*v)),
            Value::I16(v) => args.add(i64::from(*v)),
            Value::I32(v) => args.add(i64::from(*v)),
            Value::I64(v) => args.add(*v),
            Value::F32(v) => args.add(f64::from(*v)),
            Value::F64(v) => args.add(*v),
            Value::Text(v) => args.add(v.clone()),
            Value::Bytes(v) => args.add(v.clone()),
            #[cfg(feature = "uuid")]
            Value::Uuid(v) => args.add(v.hyphenated().to_string()),
            #[cfg(feature = "chrono")]
            Value::Date(v) => args.add(chrono_text::format_date(v)),
            #[cfg(feature = "chrono")]
            Value::Time(v) => args.add(chrono_text::format_time(v)),
            #[cfg(feature = "chrono")]
            Value::Timestamp(v) => args.add(chrono_text::format_timestamp(v)),
            #[cfg(feature = "chrono")]
            Value::TimestampTz(v) => args.add(chrono_text::format_timestamp_tz(v)),
            Value::Json(v) => args.add(v.to_string()),
            other => Err(format!("unsupported parameter value {other:?}").into()),
        }
    }

    /// The value's storage class (SQLite is dynamically typed).
    fn read(row: &SqliteRow, index: usize, want: SqlType) -> Result<Value, RowError> {
        let raw = row.try_get_raw(index).map_err(driver(index))?;
        if raw.is_null() {
            return Ok(Value::Null(want));
        }

        let stored = match raw.type_info().name() {
            "INTEGER" => Stored::Integer(row.try_get_unchecked(index).map_err(driver(index))?),
            "REAL" => Stored::Real(row.try_get_unchecked(index).map_err(driver(index))?),
            "BLOB" => Stored::Blob(row.try_get_unchecked(index).map_err(driver(index))?),
            _ => Stored::Text(row.try_get_unchecked(index).map_err(driver(index))?),
        };
        convert(&stored, want, index)
    }

    crate::sqlx_io!(sqlx::Sqlite);

    async fn script(conn: &mut sqlx::SqliteConnection, sql: &str) -> Result<(), sqlx::Error> {
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(&mut *conn)
            .await
            .map(drop)
    }
}

/// A non-null SQLite value, by storage class.
#[derive(Debug)]
enum Stored {
    Integer(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

/// `stored` as `want`, with the conversions `rupa-driver-rusqlite` makes.
fn convert(stored: &Stored, want: SqlType, index: usize) -> Result<Value, RowError> {
    let bad = || {
        RowError::Driver(format!(
            "column {index}: cannot read {stored:?} as {want:?}"
        ))
    };
    Ok(match (want, stored) {
        (SqlType::Bool, Stored::Integer(n)) => Value::Bool(*n != 0),
        (SqlType::I16 | SqlType::I32 | SqlType::I64, Stored::Integer(n)) => int(*n, want, index)?,
        (SqlType::F32, Stored::Real(f)) => Value::F32(*f as f32),
        (SqlType::F32, Stored::Integer(n)) => Value::F32(*n as f32),
        (SqlType::F64, Stored::Real(f)) => Value::F64(*f),
        (SqlType::F64, Stored::Integer(n)) => Value::F64(*n as f64),
        (SqlType::Text, Stored::Text(s)) => Value::Text(s.clone()),
        (SqlType::Text, Stored::Integer(n)) => Value::Text(n.to_string()),
        (SqlType::Text, Stored::Real(f)) => Value::Text(f.to_string()),
        (SqlType::Bytes, Stored::Blob(b)) => Value::Bytes(b.clone()),
        #[cfg(feature = "uuid")]
        (SqlType::Uuid, Stored::Text(s)) => Value::Uuid(s.parse().map_err(|_| bad())?),
        #[cfg(feature = "uuid")]
        (SqlType::Uuid, Stored::Blob(b)) => {
            Value::Uuid(uuid::Uuid::from_slice(b).map_err(|_| bad())?)
        }
        #[cfg(feature = "chrono")]
        (SqlType::Date, Stored::Text(s)) => {
            Value::Date(chrono_text::parse_date(s).ok_or_else(bad)?)
        }
        #[cfg(feature = "chrono")]
        (SqlType::Time, Stored::Text(s)) => {
            Value::Time(chrono_text::parse_time(s).ok_or_else(bad)?)
        }
        #[cfg(feature = "chrono")]
        (SqlType::Timestamp, Stored::Text(s)) => {
            Value::Timestamp(chrono_text::parse_timestamp(s).ok_or_else(bad)?)
        }
        #[cfg(feature = "chrono")]
        (SqlType::TimestampTz, Stored::Text(s)) => {
            Value::TimestampTz(chrono_text::parse_timestamp_tz(s).ok_or_else(bad)?)
        }
        (SqlType::Json, Stored::Text(s)) => {
            Value::Json(serde_json::from_str(s).map_err(|_| bad())?)
        }
        (SqlType::Json, Stored::Integer(n)) => Value::Json((*n).into()),
        (SqlType::Json, Stored::Real(f)) => Value::Json(serde_json::Value::from(*f)),
        _ => return Err(bad()),
    })
}
