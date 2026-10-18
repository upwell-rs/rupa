//! Conversions between rupa [`Value`]s and MySQL protocol values.
//!
//! UUIDs and JSON travel as text; `DateTime<Utc>` is stored as a UTC
//! `DATETIME(6)`; `bool` as `TINYINT(1)`.

#[cfg(feature = "chrono")]
use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime, Timelike};
use rupa_core::{RowCursor, RowError, SqlType, Value};

use crate::MySqlError;

/// A MySQL `DATETIME` value.
#[cfg(feature = "chrono")]
fn datetime(t: &NaiveDateTime) -> mysql::Value {
    mysql::Value::Date(
        t.year() as u16,
        t.month() as u8,
        t.day() as u8,
        t.hour() as u8,
        t.minute() as u8,
        t.second() as u8,
        t.nanosecond() / 1000,
    )
}

pub(crate) fn to_mysql(v: &Value) -> Result<mysql::Value, MySqlError> {
    use mysql::Value as M;

    Ok(match v {
        Value::Null(_) => M::NULL,
        Value::Bool(b) => M::Int(i64::from(*b)),
        Value::I16(n) => M::Int(i64::from(*n)),
        Value::I32(n) => M::Int(i64::from(*n)),
        Value::I64(n) => M::Int(*n),
        Value::F32(n) => M::Float(*n),
        Value::F64(n) => M::Double(*n),
        Value::Text(s) => M::Bytes(s.clone().into_bytes()),
        Value::Bytes(b) => M::Bytes(b.clone()),
        #[cfg(feature = "uuid")]
        Value::Uuid(u) => M::Bytes(u.hyphenated().to_string().into_bytes()),
        #[cfg(feature = "chrono")]
        Value::Date(d) => M::Date(d.year() as u16, d.month() as u8, d.day() as u8, 0, 0, 0, 0),
        #[cfg(feature = "chrono")]
        Value::Time(t) => M::Time(
            false,
            0,
            t.hour() as u8,
            t.minute() as u8,
            t.second() as u8,
            t.nanosecond() / 1000,
        ),
        #[cfg(feature = "chrono")]
        Value::Timestamp(t) => datetime(t),
        #[cfg(feature = "chrono")]
        Value::TimestampTz(t) => datetime(&t.naive_utc()),
        Value::Json(j) => M::Bytes(j.to_string().into_bytes()),
        other => {
            return Err(MySqlError::Bind(format!(
                "{other:?}: enable the driver's matching feature"
            )));
        }
    })
}

fn from_mysql(v: &mysql::Value, want: SqlType) -> Result<Value, RowError> {
    use mysql::Value as M;
    let bad = || RowError::Driver(format!("cannot read {v:?} as {want:?}"));
    let text = |b: &[u8]| String::from_utf8(b.to_vec()).map_err(|_| bad());
    let int = |v: &M| -> Result<i64, RowError> {
        match v {
            M::Int(n) => Ok(*n),
            M::UInt(n) => i64::try_from(*n).map_err(|_| bad()),
            M::Bytes(b) => text(b)?.parse().map_err(|_| bad()),
            _ => Err(bad()),
        }
    };
    #[cfg(feature = "chrono")]
    let datetime = |v: &M| match v {
        M::Date(y, mo, d, h, mi, s, us) => {
            NaiveDate::from_ymd_opt(i32::from(*y), u32::from(*mo), u32::from(*d))
                .and_then(|d| {
                    d.and_hms_micro_opt(u32::from(*h), u32::from(*mi), u32::from(*s), *us)
                })
                .ok_or_else(bad)
        }
        _ => Err(bad()),
    };
    Ok(match (want, v) {
        (_, M::NULL) => Value::Null(want),
        (SqlType::Bool, v) => Value::Bool(int(v)? != 0),
        (SqlType::I16, v) => Value::I16(int(v)?.try_into().map_err(|_| bad())?),
        (SqlType::I32, v) => Value::I32(int(v)?.try_into().map_err(|_| bad())?),
        (SqlType::I64, v) => Value::I64(int(v)?),
        (SqlType::F32 | SqlType::F64, M::Double(f)) => Value::F64(*f),
        (SqlType::F32 | SqlType::F64, M::Float(f)) => Value::F64(f64::from(*f)),
        (SqlType::Text, M::Bytes(b)) => Value::Text(text(b)?),
        (SqlType::Bytes, M::Bytes(b)) => Value::Bytes(b.clone()),
        #[cfg(feature = "uuid")]
        (SqlType::Uuid, M::Bytes(b)) => Value::Uuid(text(b)?.parse().map_err(|_| bad())?),
        #[cfg(feature = "chrono")]
        (SqlType::Date, v) => Value::Date(datetime(v)?.date()),
        #[cfg(feature = "chrono")]
        (SqlType::Time, M::Time(_, _, h, m, s, us)) => Value::Time(
            NaiveTime::from_hms_micro_opt(u32::from(*h), u32::from(*m), u32::from(*s), *us)
                .ok_or_else(bad)?,
        ),
        #[cfg(feature = "chrono")]
        (SqlType::Timestamp, v) => Value::Timestamp(datetime(v)?),
        #[cfg(feature = "chrono")]
        (SqlType::TimestampTz, v) => Value::TimestampTz(datetime(v)?.and_utc()),
        (SqlType::Json, M::Bytes(b)) => Value::Json(serde_json::from_slice(b).map_err(|_| bad())?),
        _ => return Err(bad()),
    })
}

struct MyRow(Vec<mysql::Value>);

impl rupa_core::Row for MyRow {
    fn len(&self) -> usize {
        self.0.len()
    }
    fn get(&self, index: usize, ty: SqlType) -> Result<Value, RowError> {
        from_mysql(
            self.0.get(index).ok_or(RowError::IndexOutOfRange {
                index,
                len: self.0.len(),
            })?,
            ty,
        )
    }
}

pub(crate) struct MyRows(std::vec::IntoIter<Vec<mysql::Value>>, Option<MyRow>);

impl MyRows {
    pub(crate) fn new(rows: Vec<Vec<mysql::Value>>) -> Self {
        Self(rows.into_iter(), None)
    }
}

impl RowCursor for MyRows {
    fn next_row(&mut self) -> Option<Result<&dyn rupa_core::Row, RowError>> {
        self.1 = Some(MyRow(self.0.next()?));
        self.1.as_ref().map(|r| Ok(r as &dyn rupa_core::Row))
    }
}
