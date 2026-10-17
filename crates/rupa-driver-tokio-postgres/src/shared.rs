//! Value and row conversion, the statement cache, and the error type, shared
//! with the sync driver (`postgres::Row`, `postgres::Statement` and
//! `postgres::Error` are re-exports of tokio-postgres').

use std::collections::HashMap;
use std::error::Error as StdError;
use std::fmt;

use bytes::BytesMut;
use postgres_types::{IsNull, ToSql, Type, to_sql_checked};
use rupa_core::exec::ExecError;
use rupa_core::row::RowCursor;
use rupa_core::{ResultError, RowError, SqlType, TxError, Value};
use rupa_sql::RenderError;

/// Everything that can go wrong running a statement on Postgres.
#[derive(Debug)]
#[non_exhaustive]
pub enum PgError {
    /// The statement could not be rendered as Postgres SQL.
    Render(RenderError),
    /// Postgres (or the connection) reported an error.
    Db(tokio_postgres::Error),
    /// The rows did not fit the query's result type.
    Result(ResultError),
    /// The transaction API was misused.
    Tx(TxError),
    /// No pooled connection could be obtained (e.g. a timeout).
    Pool(Box<dyn StdError + Send + Sync>),
}

impl fmt::Display for PgError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PgError::Render(e) => write!(f, "rendering failed: {e}"),
            PgError::Db(e) => e.fmt(f),
            PgError::Result(e) => e.fmt(f),
            PgError::Tx(e) => e.fmt(f),
            PgError::Pool(e) => write!(f, "connection pool: {e}"),
        }
    }
}

impl StdError for PgError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            PgError::Render(e) => Some(e),
            PgError::Db(e) => Some(e),
            PgError::Result(e) => Some(e),
            PgError::Tx(e) => Some(e),
            PgError::Pool(e) => Some(&**e),
        }
    }
}

impl From<RenderError> for PgError {
    fn from(e: RenderError) -> Self {
        PgError::Render(e)
    }
}

impl From<tokio_postgres::Error> for PgError {
    fn from(e: tokio_postgres::Error) -> Self {
        PgError::Db(e)
    }
}

impl From<TxError> for PgError {
    fn from(e: TxError) -> Self {
        PgError::Tx(e)
    }
}

impl From<ResultError> for PgError {
    fn from(e: ResultError) -> Self {
        PgError::Result(e)
    }
}

impl ExecError for PgError {
    fn result_error(&self) -> Option<&ResultError> {
        match self {
            PgError::Result(e) => Some(e),
            _ => None,
        }
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
            Value::Uuid(v) => v.to_sql_checked(ty, out),
            Value::Date(v) => v.to_sql_checked(ty, out),
            Value::Time(v) => v.to_sql_checked(ty, out),
            Value::Timestamp(v) => v.to_sql_checked(ty, out),
            Value::TimestampTz(v) => v.to_sql_checked(ty, out),
            Value::Json(v) => v.to_sql_checked(ty, out),
            other => Err(format!("unsupported parameter value {other:?}").into()),
        }
    }

    fn accepts(_: &Type) -> bool {
        // Per-value checks happen in `to_sql`.
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

impl From<PgError> for rupa_core::DynError {
    fn from(e: PgError) -> Self {
        rupa_core::DynError::new(e)
    }
}

/// Prepared statements kept per connection.
pub const STATEMENT_CACHE_CAPACITY: usize = 256;

/// Prepared statements by SQL text, per connection: each distinct query is
/// prepared once instead of on every call (which costs a round trip).
///
/// When full, the least recently used statement is evicted. Callers evict a
/// statement whose execution failed, since it may be stale (e.g. its table
/// was dropped and recreated with other column types).
#[derive(Debug, Default)]
pub struct StatementCache {
    statements: HashMap<String, (tokio_postgres::Statement, u64)>,
    clock: u64,
}

impl StatementCache {
    /// The cached statement for `sql`, marking it used.
    pub fn get(&mut self, sql: &str) -> Option<tokio_postgres::Statement> {
        self.clock += 1;
        let (statement, used) = self.statements.get_mut(sql)?;
        *used = self.clock;
        Some(statement.clone())
    }

    /// Caches `statement` for `sql`.
    pub fn insert(&mut self, sql: &str, statement: tokio_postgres::Statement) {
        if self.statements.len() >= STATEMENT_CACHE_CAPACITY
            && let Some(oldest) = self
                .statements
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(k, _)| k.clone())
        {
            self.statements.remove(&oldest);
        }
        self.clock += 1;
        self.statements
            .insert(sql.to_owned(), (statement, self.clock));
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
}
