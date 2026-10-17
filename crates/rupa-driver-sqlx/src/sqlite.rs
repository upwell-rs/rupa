//! SQLite 3.38+ on [`sqlx::Sqlite`] (feature `sqlite`; SQLite is compiled
//! in).
//!
//! Values are stored as `rupa-driver-rusqlite` stores them, so a database
//! file can be used by either driver: integers and booleans as `INTEGER`,
//! floats as `REAL`, UUIDs as hyphenated text, dates and times as ISO-8601
//! text with microseconds (`DateTime<Utc>` with a `Z`), JSON as text.
//!
//! The registry driver is `sqlx-sqlite`, for `sqlite:` URLs (read as
//! [`SqliteTarget`] reads them; an in-memory database gets a single
//! connection), with [`SqliteOptions`].
//!
//! sqlx runs each SQLite connection on its own worker thread, so statements
//! don't block the async runtime.

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use rupa_core::{RowError, SqlType, Value};
use sqlx::sqlite::{SqliteArguments, SqliteRow};
use sqlx::{Arguments as _, Row as _, TypeInfo as _, ValueRef as _};

#[cfg(feature = "pool")]
use rupa_core::config::{DbConfig, OpenMode, SqliteOptions, SqliteTarget};
#[cfg(feature = "pool")]
use rupa_core::registry::ConnectError;
#[cfg(feature = "pool")]
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode};
#[cfg(feature = "pool")]
use std::str::FromStr;

use crate::rows::{driver, int};
use crate::{Backend, BoxDynError, SqlxExecutor, SqlxTx};

/// An executor on one `SqliteConnection`.
pub type SqliteExecutor = SqlxExecutor<sqlx::Sqlite>;
/// A transaction on a [`SqliteExecutor`].
pub type SqliteTx<'t> = SqlxTx<'t, sqlx::Sqlite>;
/// A pool of SQLite connections.
#[cfg(feature = "pool")]
pub type SqlitePool = crate::SqlxPool<sqlx::Sqlite>;
/// Opens and recycles [`SqliteExecutor`]s for a [`SqlitePool`].
#[cfg(feature = "pool")]
pub type SqliteManager = crate::SqlxManager<sqlx::Sqlite>;
/// The registry driver `sqlx-sqlite`.
#[cfg(feature = "pool")]
pub type SqliteDriver = crate::SqlxDriver<sqlx::Sqlite>;

// The storage formats of `rupa-driver-rusqlite`; keep them in sync.
const TS_TZ: &str = "%Y-%m-%dT%H:%M:%S%.6fZ";
const TS: &str = "%Y-%m-%dT%H:%M:%S%.6f";
const TIME: &str = "%H:%M:%S%.6f";

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
            Value::Uuid(v) => args.add(v.hyphenated().to_string()),
            Value::Date(v) => args.add(v.format("%Y-%m-%d").to_string()),
            Value::Time(v) => args.add(v.format(TIME).to_string()),
            Value::Timestamp(v) => args.add(v.format(TS).to_string()),
            Value::TimestampTz(v) => args.add(v.format(TS_TZ).to_string()),
            Value::Json(v) => args.add(v.to_string()),
            other => Err(format!("unsupported parameter value {other:?}").into()),
        }
    }

    fn read(row: &SqliteRow, index: usize, want: SqlType) -> Result<Value, RowError> {
        let raw = row.try_get_raw(index).map_err(driver(index))?;
        if raw.is_null() {
            return Ok(Value::Null(want));
        }
        // The value's storage class (SQLite is dynamically typed).
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

    #[cfg(feature = "pool")]
    fn manager(config: &DbConfig) -> Result<crate::SqlxManager<Self>, ConnectError> {
        let SqliteOptions {
            busy_timeout,
            foreign_keys,
            journal_mode,
            statement_cache_size,
        } = config.driver.sqlite()?;
        let target = SqliteTarget::parse(&config.url).map_err(ConnectError::InvalidUrl)?;
        let mut options = match &target {
            SqliteTarget::Memory => SqliteConnectOptions::from_str("sqlite::memory:")
                .map_err(|e| ConnectError::InvalidUrl(e.to_string()))?,
            SqliteTarget::File { path, mode } => SqliteConnectOptions::new()
                .filename(path)
                .read_only(*mode == OpenMode::ReadOnly)
                .create_if_missing(*mode == OpenMode::ReadWriteCreate),
        };
        options = options
            .busy_timeout(busy_timeout.unwrap_or_default())
            .foreign_keys(foreign_keys);
        if let Some(mode) = &journal_mode {
            let mode =
                SqliteJournalMode::from_str(mode).map_err(|e| ConnectError::InvalidOption {
                    key: "journal_mode".into(),
                    message: e.to_string(),
                })?;
            options = options.journal_mode(mode);
        }
        if let Some(capacity) = statement_cache_size {
            options = options.statement_cache_capacity(capacity);
        }
        let manager = crate::SqlxManager::new(options);
        Ok(match target {
            // Each connection to `:memory:` would be its own database.
            SqliteTarget::Memory => manager.max_connections(Some(1)),
            SqliteTarget::File { .. } => manager,
        })
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

fn parse_timestamp(s: &str) -> Option<NaiveDateTime> {
    [
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M:%S",
    ]
    .iter()
    .find_map(|f| NaiveDateTime::parse_from_str(s.trim_end_matches('Z'), f).ok())
}

fn parse_timestamp_tz(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s)
        .map(|t| t.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            DateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S%.f%:z")
                .map(|t| t.with_timezone(&Utc))
                .ok()
        })
        .or_else(|| parse_timestamp(s).map(|t| t.and_utc()))
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
        (SqlType::Uuid, Stored::Text(s)) => Value::Uuid(s.parse().map_err(|_| bad())?),
        (SqlType::Uuid, Stored::Blob(b)) => {
            Value::Uuid(uuid::Uuid::from_slice(b).map_err(|_| bad())?)
        }
        (SqlType::Date, Stored::Text(s)) => {
            Value::Date(NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| bad())?)
        }
        (SqlType::Time, Stored::Text(s)) => {
            Value::Time(NaiveTime::parse_from_str(s, "%H:%M:%S%.f").map_err(|_| bad())?)
        }
        (SqlType::Timestamp, Stored::Text(s)) => {
            Value::Timestamp(parse_timestamp(s).ok_or_else(bad)?)
        }
        (SqlType::TimestampTz, Stored::Text(s)) => {
            Value::TimestampTz(parse_timestamp_tz(s).ok_or_else(bad)?)
        }
        (SqlType::Json, Stored::Text(s)) => {
            Value::Json(serde_json::from_str(s).map_err(|_| bad())?)
        }
        (SqlType::Json, Stored::Integer(n)) => Value::Json((*n).into()),
        (SqlType::Json, Stored::Real(f)) => Value::Json(serde_json::Value::from(*f)),
        _ => return Err(bad()),
    })
}
