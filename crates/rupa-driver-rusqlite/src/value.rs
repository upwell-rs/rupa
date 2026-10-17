//! Conversions between rupa [`Value`]s and SQLite storage values; see the
//! storage table in the crate docs.

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use rupa_core::{RowCursor, RowError, SqlType, Value};
use rusqlite::types::Value as SqlValue;

const TS_TZ: &str = "%Y-%m-%dT%H:%M:%S%.6fZ";
const TS: &str = "%Y-%m-%dT%H:%M:%S%.6f";
const TIME: &str = "%H:%M:%S%.6f";

pub(crate) fn to_sql(v: &Value) -> SqlValue {
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

pub(crate) struct SqliteRows {
    rows: std::vec::IntoIter<Vec<SqlValue>>,
    current: Option<SqliteRow>,
}

impl SqliteRows {
    pub(crate) fn new(rows: Vec<Vec<SqlValue>>) -> Self {
        Self {
            rows: rows.into_iter(),
            current: None,
        }
    }
}

impl RowCursor for SqliteRows {
    fn next_row(&mut self) -> Option<Result<&dyn rupa_core::Row, RowError>> {
        self.current = Some(SqliteRow(self.rows.next()?));
        self.current.as_ref().map(|r| Ok(r as &dyn rupa_core::Row))
    }
}
