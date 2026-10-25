//! The engine protocols and the row helpers their steps share; they exist
//! only with a mode to generate steps for.

pub mod mysql;
pub mod postgres;
pub mod sqlite;

use rupa_core::{SqlType, Value};

use crate::{Checksum, Error, HistoryEntry};

/// Column types of a history row: rank, version, checksum.
pub const HISTORY_COLUMNS: [SqlType; 3] = [SqlType::I64, SqlType::Text, SqlType::Bytes];

/// The single row a control query must return.
pub fn single<E: std::error::Error>(
    rows: Vec<Vec<Value>>,
    what: &str,
) -> Result<Vec<Value>, Error<E>> {
    let mut rows = rows.into_iter();

    match (rows.next(), rows.next()) {
        (Some(row), None) => Ok(row),
        _ => Err(Error::History(format!("{what} did not return one row"))),
    }
}

/// An integer column, `None` for `NULL`.
pub fn int(value: &Value) -> Option<i64> {
    match value {
        Value::I16(v) => Some(i64::from(*v)),
        Value::I32(v) => Some(i64::from(*v)),
        Value::I64(v) => Some(*v),
        Value::Bool(v) => Some(i64::from(*v)),
        _ => None,
    }
}

/// A text column, `None` for `NULL`.
pub fn text(value: Value) -> Option<String> {
    match value {
        Value::Text(v) => Some(v),
        _ => None,
    }
}

/// Decodes history rows.
pub fn decode_history<E: std::error::Error>(
    rows: Vec<Vec<Value>>,
) -> Result<Vec<HistoryEntry>, Error<E>> {
    rows.into_iter()
        .map(|row| {
            let [rank, Value::Text(version), Value::Bytes(checksum)] = <[Value; 3]>::try_from(row)
                .map_err(|_| Error::History("invalid history row".into()))?
            else {
                return Err(Error::History("invalid history row".into()));
            };
            let installation_rank = int(&rank)
                .and_then(|rank| u64::try_from(rank).ok())
                .filter(|rank| *rank > 0)
                .ok_or_else(|| Error::History("invalid installation rank".into()))?;
            let bytes = checksum
                .try_into()
                .map_err(|_| Error::History("invalid stored checksum length".into()))?;

            Ok(HistoryEntry {
                installation_rank,
                version,
                checksum: Checksum::from_bytes(bytes),
            })
        })
        .collect()
}
