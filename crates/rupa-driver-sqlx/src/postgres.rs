//! Postgres on [`sqlx::Postgres`] (feature `postgres`).
//!
//! Values are bound with their own Postgres types (`bigint` for `i64`,
//! `jsonb` for JSON, `timestamptz` for `DateTime<Utc>`); Postgres casts them
//! to the column's type where an implicit or assignment cast exists, e.g. an
//! `i64` into an `integer` column. Integer columns are read at any width,
//! narrowed when the value fits.
//!
//! The registry driver is `sqlx-postgres`, for `postgres:` and
//! `postgresql:` URLs, with [`PostgresOptions`]. TLS is configured in the URL
//! (`?sslmode=require`) and needs a `tls-*` feature.

use rupa_core::{RowError, SqlType, Value};
use sqlx::postgres::{PgArguments, PgRow};
use sqlx::{Arguments as _, Column as _, Row as _, TypeInfo as _};

#[cfg(feature = "pool")]
use rupa_core::config::{DbConfig, PostgresOptions};
#[cfg(feature = "pool")]
use rupa_core::registry::ConnectError;
#[cfg(feature = "pool")]
use sqlx::postgres::PgConnectOptions;
#[cfg(feature = "pool")]
use std::str::FromStr;

use crate::rows::{driver, int};
use crate::{Backend, BoxDynError, SqlxExecutor, SqlxTx};

/// An executor on one `PgConnection`.
pub type PgExecutor = SqlxExecutor<sqlx::Postgres>;
/// A transaction on a [`PgExecutor`].
pub type PgTx<'t> = SqlxTx<'t, sqlx::Postgres>;
/// A pool of Postgres connections.
#[cfg(feature = "pool")]
pub type PgPool = crate::SqlxPool<sqlx::Postgres>;
/// Opens and recycles [`PgExecutor`]s for a [`PgPool`].
#[cfg(feature = "pool")]
pub type PgManager = crate::SqlxManager<sqlx::Postgres>;
/// The registry driver `sqlx-postgres`.
#[cfg(feature = "pool")]
pub type PgDriver = crate::SqlxDriver<sqlx::Postgres>;

impl crate::sealed::Sealed for sqlx::Postgres {}

impl Backend for sqlx::Postgres {
    type Dialect = rupa_core::Postgres;
    const DRIVER: &'static str = "sqlx-postgres";
    const SCHEMES: &'static [&'static str] = &["postgres", "postgresql"];

    fn bind(args: &mut PgArguments, value: &Value) -> Result<(), BoxDynError> {
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
            Value::Uuid(v) => args.add(*v),
            Value::Date(v) => args.add(*v),
            Value::Time(v) => args.add(*v),
            Value::Timestamp(v) => args.add(*v),
            Value::TimestampTz(v) => args.add(*v),
            Value::Json(v) => args.add(sqlx::types::Json(v)),
            other => Err(format!("unsupported parameter value {other:?}").into()),
        }
    }

    fn read(row: &PgRow, index: usize, want: SqlType) -> Result<Value, RowError> {
        let column_type = row.column(index).type_info().name().to_owned();
        let mismatch = || {
            RowError::Driver(format!(
                "column {index} has type {column_type}, which cannot be read as {want:?}"
            ))
        };
        macro_rules! get {
            ($t:ty) => {
                row.try_get::<Option<$t>, _>(index).map_err(driver(index))?
            };
        }
        let null = Value::Null(want);
        Ok(match want {
            SqlType::Bool => get!(bool).map_or(null, Value::Bool),
            SqlType::I16 | SqlType::I32 | SqlType::I64 => {
                let n = match column_type.as_str() {
                    "INT2" => get!(i16).map(i64::from),
                    "INT4" => get!(i32).map(i64::from),
                    "INT8" => get!(i64),
                    _ => return Err(mismatch()),
                };
                match n {
                    None => null,
                    Some(n) => int(n, want, index)?,
                }
            }
            SqlType::F32 => get!(f32).map_or(null, Value::F32),
            SqlType::F64 => match column_type.as_str() {
                "FLOAT4" => get!(f32).map_or(null, |v| Value::F64(f64::from(v))),
                _ => get!(f64).map_or(null, Value::F64),
            },
            SqlType::Text => get!(String).map_or(null, Value::Text),
            SqlType::Bytes => get!(Vec<u8>).map_or(null, Value::Bytes),
            SqlType::Uuid => get!(uuid::Uuid).map_or(null, Value::Uuid),
            SqlType::Date => get!(chrono::NaiveDate).map_or(null, Value::Date),
            SqlType::Time => get!(chrono::NaiveTime).map_or(null, Value::Time),
            SqlType::Timestamp => get!(chrono::NaiveDateTime).map_or(null, Value::Timestamp),
            SqlType::TimestampTz => {
                get!(chrono::DateTime<chrono::Utc>).map_or(null, Value::TimestampTz)
            }
            SqlType::Json => get!(serde_json::Value).map_or(null, Value::Json),
        })
    }

    crate::sqlx_io!(sqlx::Postgres);

    async fn script(conn: &mut sqlx::PgConnection, sql: &str) -> Result<(), sqlx::Error> {
        // The simple-query protocol runs several statements in one round trip.
        sqlx::raw_sql(sqlx::AssertSqlSafe(sql))
            .execute(&mut *conn)
            .await
            .map(drop)
    }

    #[cfg(feature = "pool")]
    fn manager(config: &DbConfig) -> Result<crate::SqlxManager<Self>, ConnectError> {
        let PostgresOptions {
            application_name,
            connect_timeout,
            statement_cache_size,
        } = config.driver.postgres()?;
        let mut options = PgConnectOptions::from_str(&config.url)
            .map_err(|e| ConnectError::InvalidUrl(e.to_string()))?;
        if let Some(name) = &application_name {
            options = options.application_name(name);
        }
        if let Some(capacity) = statement_cache_size {
            options = options.statement_cache_capacity(capacity);
        }
        Ok(crate::SqlxManager::new(options).connect_timeout(connect_timeout))
    }
}

/// A `NULL` of the Postgres type matching `ty`.
fn null(args: &mut PgArguments, ty: SqlType) -> Result<(), BoxDynError> {
    match ty {
        SqlType::Bool => args.add(None::<bool>),
        SqlType::I16 => args.add(None::<i16>),
        SqlType::I32 => args.add(None::<i32>),
        SqlType::I64 => args.add(None::<i64>),
        SqlType::F32 => args.add(None::<f32>),
        SqlType::F64 => args.add(None::<f64>),
        SqlType::Text => args.add(None::<&str>),
        SqlType::Bytes => args.add(None::<&[u8]>),
        SqlType::Uuid => args.add(None::<uuid::Uuid>),
        SqlType::Date => args.add(None::<chrono::NaiveDate>),
        SqlType::Time => args.add(None::<chrono::NaiveTime>),
        SqlType::Timestamp => args.add(None::<chrono::NaiveDateTime>),
        SqlType::TimestampTz => args.add(None::<chrono::DateTime<chrono::Utc>>),
        SqlType::Json => args.add(None::<sqlx::types::Json<serde_json::Value>>),
    }
}
