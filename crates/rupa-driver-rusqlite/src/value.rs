//! Conversions between rupa [`Value`]s and SQLite storage values; see the
//! storage table in the crate docs.

#[cfg(feature = "chrono")]
use rupa_core::chrono_text;
use rupa_core::{RowCursor, RowError, SqlType, Value};
use rusqlite::ToSql;
use rusqlite::types::{ToSqlOutput, Value as SqlValue, ValueRef};

/// A [`Value`] bound as a SQLite parameter. Text and blobs are borrowed, not
/// copied.
pub(crate) struct Param<'a>(pub(crate) &'a Value);

impl ToSql for Param<'_> {
    fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
        let owned = |v: SqlValue| Ok(ToSqlOutput::Owned(v));

        match self.0 {
            Value::Null(_) => owned(SqlValue::Null),
            Value::Bool(b) => owned(SqlValue::Integer(i64::from(*b))),
            Value::I16(n) => owned(SqlValue::Integer(i64::from(*n))),
            Value::I32(n) => owned(SqlValue::Integer(i64::from(*n))),
            Value::I64(n) => owned(SqlValue::Integer(*n)),
            Value::F32(n) => owned(SqlValue::Real(f64::from(*n))),
            Value::F64(n) => owned(SqlValue::Real(*n)),
            Value::Text(s) => Ok(ToSqlOutput::Borrowed(ValueRef::Text(s.as_bytes()))),
            Value::Bytes(b) => Ok(ToSqlOutput::Borrowed(ValueRef::Blob(b))),
            #[cfg(feature = "uuid")]
            Value::Uuid(u) => owned(SqlValue::Text(u.hyphenated().to_string())),
            #[cfg(feature = "chrono")]
            Value::Date(d) => owned(SqlValue::Text(chrono_text::format_date(d))),
            #[cfg(feature = "chrono")]
            Value::Time(t) => owned(SqlValue::Text(chrono_text::format_time(t))),
            #[cfg(feature = "chrono")]
            Value::Timestamp(t) => owned(SqlValue::Text(chrono_text::format_timestamp(t))),
            #[cfg(feature = "chrono")]
            Value::TimestampTz(t) => owned(SqlValue::Text(chrono_text::format_timestamp_tz(t))),
            Value::Json(j) => owned(SqlValue::Text(j.to_string())),
            other => Err(rusqlite::Error::ToSqlConversionFailure(
                format!("cannot bind {other:?}: enable the driver's matching feature").into(),
            )),
        }
    }
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
        #[cfg(feature = "uuid")]
        (SqlType::Uuid, SqlValue::Text(s)) => Value::Uuid(s.parse().map_err(|_| bad())?),
        #[cfg(feature = "uuid")]
        (SqlType::Uuid, SqlValue::Blob(b)) => {
            Value::Uuid(uuid::Uuid::from_slice(b).map_err(|_| bad())?)
        }
        #[cfg(feature = "chrono")]
        (SqlType::Date, SqlValue::Text(s)) => {
            Value::Date(chrono_text::parse_date(s).ok_or_else(bad)?)
        }
        #[cfg(feature = "chrono")]
        (SqlType::Time, SqlValue::Text(s)) => {
            Value::Time(chrono_text::parse_time(s).ok_or_else(bad)?)
        }
        #[cfg(feature = "chrono")]
        (SqlType::Timestamp, SqlValue::Text(s)) => {
            Value::Timestamp(chrono_text::parse_timestamp(s).ok_or_else(bad)?)
        }
        #[cfg(feature = "chrono")]
        (SqlType::TimestampTz, SqlValue::Text(s)) => {
            Value::TimestampTz(chrono_text::parse_timestamp_tz(s).ok_or_else(bad)?)
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
