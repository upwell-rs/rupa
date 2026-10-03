//! Synchronous RUPA executor on [`rusqlite`] (SQLite 3.38+).
//!
//! # Storage formats
//!
//! SQLite has no date, UUID or JSON types; values are stored as text in
//! fixed formats, so that text order is value order:
//!
//! | value | stored as |
//! |---|---|
//! | `bool` | `INTEGER` 0/1 |
//! | `DateTime<Utc>` | `TEXT` `YYYY-MM-DDTHH:MM:SS.ffffffZ` (UTC, always 6 digits) |
//! | `NaiveDateTime` | `TEXT` `YYYY-MM-DDTHH:MM:SS.ffffff` |
//! | `NaiveDate` / `NaiveTime` | `TEXT` `YYYY-MM-DD` / `HH:MM:SS.ffffff` |
//! | `Uuid` | `TEXT`, hyphenated |
//! | JSON | `TEXT` |
//!
//! Reading also accepts other common forms (space separator, offsets,
//! 16-byte UUID blobs).
//!
//! # Semantics
//!
//! [`SqliteExecutor::new`] enables `PRAGMA case_sensitive_like`, so `LIKE`
//! is case-sensitive as in Postgres.
//!
//! # Transactions
//!
//! [`Transactional`]: `begin` returns a [`SqliteTx`]; nested `begin` opens a
//! savepoint. A dropped `SqliteTx` rolls back immediately. If that fails,
//! the executor is marked dirty and the rollback is retried before the next
//! statement. Read-only transactions are refused: SQLite cannot enforce them.

use std::fmt;

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use rupa_core::exec::{ExecError, Executor, Outcome};
use rupa_core::ir::{Statement, TxStatement};
use rupa_core::query::Expect;
use rupa_core::tx::{Transaction, Transactional, TxOptions};
use rupa_core::{ResultError, RowCursor, RowError, SqlType, Sqlite, TxError, Value};
use rupa_sql::RenderError;
use rusqlite::types::Value as SqlValue;

#[derive(Debug)]
#[non_exhaustive]
pub enum SqliteError {
    Render(RenderError),
    Db(rusqlite::Error),
    Result(ResultError),
    Tx(TxError),
}

impl fmt::Display for SqliteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SqliteError::Render(e) => write!(f, "rendering failed: {e}"),
            SqliteError::Db(e) => e.fmt(f),
            SqliteError::Result(e) => e.fmt(f),
            SqliteError::Tx(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for SqliteError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            SqliteError::Render(e) => Some(e),
            SqliteError::Db(e) => Some(e),
            SqliteError::Result(e) => Some(e),
            SqliteError::Tx(e) => Some(e),
        }
    }
}

impl From<RenderError> for SqliteError {
    fn from(e: RenderError) -> Self {
        SqliteError::Render(e)
    }
}

impl From<rusqlite::Error> for SqliteError {
    fn from(e: rusqlite::Error) -> Self {
        SqliteError::Db(e)
    }
}

impl From<ResultError> for SqliteError {
    fn from(e: ResultError) -> Self {
        SqliteError::Result(e)
    }
}

impl ExecError for SqliteError {
    fn result_error(&self) -> Option<&ResultError> {
        match self {
            SqliteError::Result(e) => Some(e),
            _ => None,
        }
    }
}

impl From<SqliteError> for rupa_core::exec::DynError {
    fn from(e: SqliteError) -> Self {
        rupa_core::exec::DynError::new(e)
    }
}

// ---------------------------------------------------------------------------
// Values
// ---------------------------------------------------------------------------

const TS_TZ: &str = "%Y-%m-%dT%H:%M:%S%.6fZ";
const TS: &str = "%Y-%m-%dT%H:%M:%S%.6f";
const TIME: &str = "%H:%M:%S%.6f";

fn to_sql(v: &Value) -> SqlValue {
    match v {
        Value::Null(_) => SqlValue::Null,
        Value::Bool(b) => SqlValue::Integer(i64::from(*b)),
        Value::I16(n) => SqlValue::Integer(i64::from(*n)),
        Value::I32(n) => SqlValue::Integer(i64::from(*n)),
        Value::I64(n) => SqlValue::Integer(*n),
        Value::F32(n) => SqlValue::Real(f64::from(*n)),
        Value::F64(n) => SqlValue::Real(*n),
        Value::Text(s) => SqlValue::Text(s.clone()),
        Value::Bytes(b) => SqlValue::Blob(b.clone()),
        Value::Uuid(u) => SqlValue::Text(u.hyphenated().to_string()),
        Value::Date(d) => SqlValue::Text(d.format("%Y-%m-%d").to_string()),
        Value::Time(t) => SqlValue::Text(t.format(TIME).to_string()),
        Value::Timestamp(t) => SqlValue::Text(t.format(TS).to_string()),
        Value::TimestampTz(t) => SqlValue::Text(t.format(TS_TZ).to_string()),
        Value::Json(j) => SqlValue::Text(j.to_string()),
        other => SqlValue::Text(format!("{other:?}")),
    }
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

/// Converts a stored value to the requested SQL type.
fn from_sql(v: &SqlValue, want: SqlType, index: usize) -> Result<Value, RowError> {
    let bad = || RowError::Driver(format!("column {index}: cannot read {v:?} as {want:?}"));
    let int = |n: i64| -> Result<Value, RowError> {
        Ok(match want {
            SqlType::I16 => Value::I16(n.try_into().map_err(|_| bad())?),
            SqlType::I32 => Value::I32(n.try_into().map_err(|_| bad())?),
            _ => Value::I64(n),
        })
    };
    Ok(match (want, v) {
        (_, SqlValue::Null) => Value::Null(want),
        (SqlType::Bool, SqlValue::Integer(n)) => Value::Bool(*n != 0),
        (SqlType::I16 | SqlType::I32 | SqlType::I64, SqlValue::Integer(n)) => int(*n)?,
        (SqlType::F32, SqlValue::Real(f)) => Value::F32(*f as f32),
        (SqlType::F32, SqlValue::Integer(n)) => Value::F32(*n as f32),
        (SqlType::F64, SqlValue::Real(f)) => Value::F64(*f),
        (SqlType::F64, SqlValue::Integer(n)) => Value::F64(*n as f64),
        (SqlType::Text, SqlValue::Text(s)) => Value::Text(s.clone()),
        (SqlType::Text, SqlValue::Integer(n)) => Value::Text(n.to_string()),
        (SqlType::Text, SqlValue::Real(f)) => Value::Text(f.to_string()),
        (SqlType::Bytes, SqlValue::Blob(b)) => Value::Bytes(b.clone()),
        (SqlType::Uuid, SqlValue::Text(s)) => Value::Uuid(s.parse().map_err(|_| bad())?),
        (SqlType::Uuid, SqlValue::Blob(b)) => {
            Value::Uuid(uuid::Uuid::from_slice(b).map_err(|_| bad())?)
        }
        (SqlType::Date, SqlValue::Text(s)) => {
            Value::Date(NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| bad())?)
        }
        (SqlType::Time, SqlValue::Text(s)) => {
            Value::Time(NaiveTime::parse_from_str(s, "%H:%M:%S%.f").map_err(|_| bad())?)
        }
        (SqlType::Timestamp, SqlValue::Text(s)) => {
            Value::Timestamp(parse_timestamp(s).ok_or_else(bad)?)
        }
        (SqlType::TimestampTz, SqlValue::Text(s)) => {
            Value::TimestampTz(parse_timestamp_tz(s).ok_or_else(bad)?)
        }
        (SqlType::Json, SqlValue::Text(s)) => {
            Value::Json(serde_json::from_str(s).map_err(|_| bad())?)
        }
        (SqlType::Json, SqlValue::Integer(n)) => Value::Json((*n).into()),
        (SqlType::Json, SqlValue::Real(f)) => Value::Json(serde_json::Value::from(*f)),
        _ => return Err(bad()),
    })
}

struct SqliteRow(Vec<SqlValue>);

impl rupa_core::Row for SqliteRow {
    fn len(&self) -> usize {
        self.0.len()
    }
    fn get(&self, index: usize, ty: SqlType) -> Result<Value, RowError> {
        let v = self.0.get(index).ok_or(RowError::IndexOutOfRange {
            index,
            len: self.0.len(),
        })?;
        from_sql(v, ty, index)
    }
}

struct SqliteRows {
    rows: std::vec::IntoIter<Vec<SqlValue>>,
    current: Option<SqliteRow>,
}

impl RowCursor for SqliteRows {
    fn next_row(&mut self) -> Option<Result<&dyn rupa_core::Row, RowError>> {
        self.current = Some(SqliteRow(self.rows.next()?));
        self.current.as_ref().map(|r| Ok(r as &dyn rupa_core::Row))
    }
}

// ---------------------------------------------------------------------------
// Executor
// ---------------------------------------------------------------------------

/// Executes queries on a `rusqlite::Connection`.
pub struct SqliteExecutor {
    conn: rusqlite::Connection,
    dialect: Sqlite,
    /// A rollback a dropped transaction could not complete (`0` = the whole
    /// transaction, otherwise a savepoint depth).
    pending_rollback: Option<u32>,
}

impl SqliteExecutor {
    /// Wraps a connection, enabling Postgres-compatible `LIKE` semantics.
    pub fn new(conn: rusqlite::Connection) -> Result<Self, SqliteError> {
        conn.pragma_update(None, "case_sensitive_like", true)?;
        Ok(Self {
            conn,
            dialect: Sqlite,
            pending_rollback: None,
        })
    }

    pub fn open_in_memory() -> Result<Self, SqliteError> {
        Self::new(rusqlite::Connection::open_in_memory()?)
    }

    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, SqliteError> {
        Self::new(rusqlite::Connection::open(path)?)
    }

    /// The underlying connection. Statements sent through it directly bypass
    /// the pending-rollback check; call [`clean`](Self::clean) first.
    pub fn connection(&mut self) -> &mut rusqlite::Connection {
        &mut self.conn
    }

    pub fn into_inner(self) -> rusqlite::Connection {
        self.conn
    }

    pub fn is_dirty(&self) -> bool {
        self.pending_rollback.is_some()
    }

    /// Sends the rollback owed by a dropped transaction, if any.
    pub fn clean(&mut self) -> Result<(), SqliteError> {
        if let Some(depth) = self.pending_rollback {
            self.conn
                .execute_batch(&rupa_sql::render_tx(&rollback_to(depth), &self.dialect)?)?;
            self.pending_rollback = None;
        }
        Ok(())
    }

    fn mark_dirty(&mut self, depth: u32) {
        self.pending_rollback = Some(self.pending_rollback.map_or(depth, |d| d.min(depth)));
    }

    fn control(&mut self, statement: TxStatement) -> Result<(), SqliteError> {
        self.clean()?;
        let sql = rupa_sql::render_tx(&statement, &self.dialect)?;
        Ok(self.conn.execute_batch(&sql)?)
    }

    fn run(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, SqliteError> {
        self.clean()?;
        let rendered = rupa_sql::render(statement, &self.dialect)?;
        let params = rusqlite::params_from_iter(rendered.params.iter().map(to_sql));
        let mut stmt = self.conn.prepare(&rendered.sql)?;
        if expect == Expect::Affected {
            return Ok(Outcome::Affected(stmt.execute(params)? as u64));
        }
        let limit = expect.fetch_limit().unwrap_or(usize::MAX);
        let width = stmt.column_count();
        let mut rows = stmt.query(params)?;
        let mut out = Vec::new();
        while out.len() < limit {
            let Some(row) = rows.next()? else { break };
            out.push(
                (0..width)
                    .map(|i| row.get::<_, SqlValue>(i))
                    .collect::<Result<Vec<_>, _>>()?,
            );
        }
        Ok(Outcome::Rows(Box::new(SqliteRows {
            rows: out.into_iter(),
            current: None,
        })))
    }
}

fn rollback_to(depth: u32) -> TxStatement {
    match depth {
        0 => TxStatement::Rollback,
        d => TxStatement::RollbackToSavepoint(d),
    }
}

impl Executor for SqliteExecutor {
    type Dialect = Sqlite;
    type Error = SqliteError;

    fn dialect(&self) -> &Sqlite {
        &self.dialect
    }

    fn execute(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, SqliteError> {
        self.run(statement, expect)
    }
}

impl Transactional for SqliteExecutor {
    type Tx<'t>
        = SqliteTx<'t>
    where
        Self: 't;

    fn begin_with(&mut self, options: TxOptions) -> Result<SqliteTx<'_>, SqliteError> {
        self.control(TxStatement::Begin(options))?;
        Ok(SqliteTx {
            exec: self,
            depth: 0,
            done: false,
        })
    }
}

/// An open transaction (`depth == 0`) or savepoint (`depth > 0`). Rolled back
/// when dropped without `commit`.
pub struct SqliteTx<'t> {
    exec: &'t mut SqliteExecutor,
    depth: u32,
    done: bool,
}

impl SqliteTx<'_> {
    fn finish(&mut self, commit: bool) -> Result<(), SqliteError> {
        let statement = match (commit, self.depth) {
            (true, 0) => TxStatement::Commit,
            (true, d) => TxStatement::ReleaseSavepoint(d),
            (false, d) => rollback_to(d),
        };
        self.exec.control(statement)?;
        self.done = true;
        Ok(())
    }
}

impl Drop for SqliteTx<'_> {
    fn drop(&mut self) {
        if !self.done && self.finish(false).is_err() {
            self.exec.mark_dirty(self.depth);
        }
    }
}

impl Executor for SqliteTx<'_> {
    type Dialect = Sqlite;
    type Error = SqliteError;

    fn dialect(&self) -> &Sqlite {
        &self.exec.dialect
    }

    fn execute(&mut self, statement: &Statement, expect: Expect) -> Result<Outcome, SqliteError> {
        self.exec.run(statement, expect)
    }
}

impl Transaction for SqliteTx<'_> {
    fn commit(mut self) -> Result<(), SqliteError> {
        self.finish(true)
    }

    fn rollback(mut self) -> Result<(), SqliteError> {
        self.finish(false)
    }
}

impl Transactional for SqliteTx<'_> {
    type Tx<'s>
        = SqliteTx<'s>
    where
        Self: 's;

    fn begin_with(&mut self, options: TxOptions) -> Result<SqliteTx<'_>, SqliteError> {
        if !options.is_default() {
            return Err(SqliteError::Tx(TxError::OptionsOnNested));
        }
        let depth = self.depth + 1;
        self.exec.control(TxStatement::Savepoint(depth))?;
        Ok(SqliteTx {
            exec: &mut *self.exec,
            depth,
            done: false,
        })
    }
}
