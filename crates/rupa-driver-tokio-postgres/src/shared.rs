//! Value and row conversion, the statement cache, and the error type, shared
//! with the sync driver (`postgres::Row`, `postgres::Statement` and
//! `postgres::Error` are re-exports of tokio-postgres').

use std::error::Error as StdError;
use std::sync::Arc;

use bytes::BytesMut;
use hashlink::LruCache;
use postgres_types::{IsNull, ToSql, Type, to_sql_checked};
use rupa_core::row::RowCursor;
use rupa_core::{RowError, SqlType, Value};

rupa_sql::driver_error! {
    /// Everything that can go wrong running a statement on Postgres.
    pub enum PgError {
        /// Postgres (or the connection) reported an error.
        Db(tokio_postgres::Error),
        /// A migration connection was used outside a runner-owned artifact transaction.
        #[cfg(feature = "migration-protocol")]
        Migration(rupa_migrate::StateError),
    }
}

/// A [`Value`] bound as a Postgres parameter. Parameter types are inferred by
/// the server; integers are narrowed or widened to the inferred width when the
/// value fits, so an `i64` can be compared with an `integer` column.
#[derive(Debug)]
pub struct Param<'a>(pub &'a Value);

type BoxError = Box<dyn StdError + Sync + Send>;

fn int(v: i64, ty: &Type, out: &mut BytesMut) -> Result<IsNull, BoxError> {
    match *ty {
        Type::INT2 => i16::try_from(v)?.to_sql(ty, out),
        Type::INT4 => i32::try_from(v)?.to_sql(ty, out),
        Type::INT8 => v.to_sql(ty, out),
        _ => Err(format!("cannot bind an integer as {ty}").into()),
    }
}

impl ToSql for Param<'_> {
    fn to_sql(&self, ty: &Type, out: &mut BytesMut) -> Result<IsNull, BoxError> {
        match self.0 {
            Value::Null(_) => Ok(IsNull::Yes),
            Value::Bool(v) => v.to_sql_checked(ty, out),
            Value::I16(v) => int(i64::from(*v), ty, out),
            Value::I32(v) => int(i64::from(*v), ty, out),
            Value::I64(v) => int(*v, ty, out),
            Value::F32(v) if *ty == Type::FLOAT8 => f64::from(*v).to_sql(ty, out),
            Value::F32(v) => v.to_sql_checked(ty, out),
            Value::F64(v) => v.to_sql_checked(ty, out),
            Value::Text(v) => v.to_sql_checked(ty, out),
            Value::Bytes(v) => v.to_sql_checked(ty, out),
            #[cfg(feature = "uuid")]
            Value::Uuid(v) => v.to_sql_checked(ty, out),
            #[cfg(feature = "chrono")]
            Value::Date(v) => v.to_sql_checked(ty, out),
            #[cfg(feature = "chrono")]
            Value::Time(v) => v.to_sql_checked(ty, out),
            #[cfg(feature = "chrono")]
            Value::Timestamp(v) => v.to_sql_checked(ty, out),
            #[cfg(feature = "chrono")]
            Value::TimestampTz(v) => v.to_sql_checked(ty, out),
            Value::Json(v) => v.to_sql_checked(ty, out),
            other => Err(format!("unsupported parameter value {other:?}").into()),
        }
    }

    /// Per-value checks happen in `to_sql`.
    fn accepts(_: &Type) -> bool {
        true
    }

    to_sql_checked!();
}

/// Reads column `index` as `want`, converting from the column's actual type
/// where that is lossless (integer widening, or narrowing when the value fits).
pub fn read(row: &tokio_postgres::Row, index: usize, want: SqlType) -> Result<Value, RowError> {
    let len = row.len();
    let ty = row
        .columns()
        .get(index)
        .ok_or(RowError::IndexOutOfRange { index, len })?
        .type_()
        .clone();
    let driver = |e: tokio_postgres::Error| RowError::Driver(e.to_string());
    let mismatch = || {
        RowError::Driver(format!(
            "column {index} has type {ty}, which cannot be read as {want:?}"
        ))
    };
    macro_rules! get {
        ($t:ty) => {
            row.try_get::<_, Option<$t>>(index).map_err(driver)?
        };
    }
    let null = Value::Null(want);
    Ok(match want {
        SqlType::Bool => get!(bool).map_or(null, Value::Bool),
        SqlType::I16 | SqlType::I32 | SqlType::I64 => {
            let n: Option<i64> = match ty {
                Type::INT2 => get!(i16).map(i64::from),
                Type::INT4 => get!(i32).map(i64::from),
                Type::INT8 => get!(i64),
                _ => return Err(mismatch()),
            };
            let out_of_range = |n: i64| {
                RowError::Driver(format!("value {n} in column {index} does not fit {want:?}"))
            };
            match n {
                None => null,
                Some(n) => match want {
                    SqlType::I16 => Value::I16(n.try_into().map_err(|_| out_of_range(n))?),
                    SqlType::I32 => Value::I32(n.try_into().map_err(|_| out_of_range(n))?),
                    _ => Value::I64(n),
                },
            }
        }
        SqlType::F32 => get!(f32).map_or(null, Value::F32),
        SqlType::F64 => match ty {
            Type::FLOAT4 => get!(f32).map_or(null, |v| Value::F64(f64::from(v))),
            _ => get!(f64).map_or(null, Value::F64),
        },
        SqlType::Text => get!(String).map_or(null, Value::Text),
        SqlType::Bytes => get!(Vec<u8>).map_or(null, Value::Bytes),
        #[cfg(feature = "uuid")]
        SqlType::Uuid => get!(uuid::Uuid).map_or(null, Value::Uuid),
        #[cfg(feature = "chrono")]
        SqlType::Date => get!(chrono::NaiveDate).map_or(null, Value::Date),
        #[cfg(feature = "chrono")]
        SqlType::Time => get!(chrono::NaiveTime).map_or(null, Value::Time),
        #[cfg(feature = "chrono")]
        SqlType::Timestamp => get!(chrono::NaiveDateTime).map_or(null, Value::Timestamp),
        #[cfg(feature = "chrono")]
        SqlType::TimestampTz => {
            get!(chrono::DateTime<chrono::Utc>).map_or(null, Value::TimestampTz)
        }
        SqlType::Json => get!(serde_json::Value).map_or(null, Value::Json),
        #[cfg(not(all(feature = "chrono", feature = "uuid")))]
        other => return Err(RowError::not_enabled(other)),
    })
}

pub struct PgRow(pub tokio_postgres::Row);

impl rupa_core::Row for PgRow {
    fn len(&self) -> usize {
        self.0.len()
    }
    fn get(&self, index: usize, ty: SqlType) -> Result<Value, RowError> {
        read(&self.0, index, ty)
    }
}

/// Cursor over buffered rows.
pub struct PgRows {
    rows: std::vec::IntoIter<tokio_postgres::Row>,
    current: Option<PgRow>,
}

impl PgRows {
    pub fn new(rows: Vec<tokio_postgres::Row>) -> Self {
        Self {
            rows: rows.into_iter(),
            current: None,
        }
    }
}

impl RowCursor for PgRows {
    fn next_row(&mut self) -> Option<Result<&dyn rupa_core::Row, RowError>> {
        self.current = Some(PgRow(self.rows.next()?));
        self.current.as_ref().map(|r| Ok(r as &dyn rupa_core::Row))
    }
}

/// Prepared statements kept per connection by default.
pub const STATEMENT_CACHE_CAPACITY: usize = 256;

/// Server-side prepared statements by SQL text, per connection: each distinct
/// query is prepared once (`Parse`) and then only executed (`Bind`/`Execute`),
/// which saves a round trip and lets Postgres reuse its plan (after a few
/// executions it may switch to a generic plan; see `plan_cache_mode`).
///
/// Bounded: when full, the least recently used statement is dropped, which
/// closes it on the server. A statement whose execution failed is evicted too,
/// since it may be stale (e.g. its table was recreated with other column
/// types). Keys share the render cache's SQL text, so caching costs no copy.
#[derive(Debug)]
pub struct StatementCache {
    statements: LruCache<Arc<str>, tokio_postgres::Statement>,
}

impl Default for StatementCache {
    fn default() -> Self {
        Self::new(STATEMENT_CACHE_CAPACITY)
    }
}

impl StatementCache {
    /// A cache for at most `capacity` statements (`0`: prepare every time).
    pub fn new(capacity: usize) -> Self {
        Self {
            statements: LruCache::new(capacity),
        }
    }

    /// The cached statement for `sql`, marking it used.
    pub fn get(&mut self, sql: &str) -> Option<tokio_postgres::Statement> {
        self.statements.get(sql).cloned()
    }

    /// Caches `statement` for `sql`.
    pub fn insert(&mut self, sql: Arc<str>, statement: tokio_postgres::Statement) {
        self.statements.insert(sql, statement);
    }

    /// Forgets the statement for `sql`.
    pub fn evict(&mut self, sql: &str) {
        self.statements.remove(sql);
    }

    /// Forgets every statement (e.g. after `DISCARD ALL`).
    pub fn clear(&mut self) {
        self.statements.clear();
    }

    /// The number of cached statements.
    pub fn len(&self) -> usize {
        self.statements.len()
    }

    /// Whether nothing is cached.
    pub fn is_empty(&self) -> bool {
        self.statements.is_empty()
    }

    /// The most statements kept.
    pub fn capacity(&self) -> usize {
        self.statements.capacity()
    }

    /// Changes the capacity, dropping the least recently used statements
    /// that no longer fit.
    pub fn set_capacity(&mut self, capacity: usize) {
        self.statements.set_capacity(capacity);
    }
}

/// The connection settings for a `postgres:` URL, with the config's
/// [`PostgresOptions`](rupa_core::config::PostgresOptions) applied.
pub fn pg_config(
    config: &rupa_core::config::DbConfig,
) -> Result<(tokio_postgres::Config, Option<usize>), rupa_core::ConnectError> {
    let mut pg = config
        .url
        .parse::<tokio_postgres::Config>()
        .map_err(|e| rupa_core::ConnectError::InvalidUrl(e.to_string()))?;
    let options = config.driver.postgres()?;

    if let Some(name) = &options.application_name {
        pg.application_name(name);
    }

    if let Some(timeout) = options.connect_timeout {
        pg.connect_timeout(timeout);
    }

    Ok((pg, options.statement_cache_size))
}

/// A TLS connector on rustls (ring, Mozilla's root certificates). The URL's
/// `sslmode` decides whether TLS is used (`prefer`, the default, tries it).
#[cfg(feature = "tls-rustls")]
pub fn rustls() -> Result<tokio_postgres_rustls::MakeRustlsConnect, rupa_core::ConnectError> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let roots = rustls::RootCertStore {
        roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
    };
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| rupa_core::ConnectError::Connect(rupa_core::DynError::other(e)))?
        .with_root_certificates(roots)
        .with_no_client_auth();

    Ok(tokio_postgres_rustls::MakeRustlsConnect::new(config))
}
