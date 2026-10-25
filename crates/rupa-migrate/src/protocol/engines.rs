//! The engine protocols and the row helpers their steps share; they exist
//! only with a mode to generate steps for.

pub mod mysql;
pub mod postgres;
pub mod sqlite;

use rupa_core::{SqlType, Value};

use crate::{
    Attempt, CatalogEntry, Checksum, Error, HistoryEntry, Journal, Policy, Provenance, Records,
    Recovery, RecoveryAction, RecoveryError,
};

/// Column types of a history row: rank, version, checksum, status.
pub const HISTORY_COLUMNS: [SqlType; 4] =
    [SqlType::I64, SqlType::Text, SqlType::Bytes, SqlType::Text];

/// Column types of an attempt row: version, checksum, status.
pub const ATTEMPT_COLUMNS: [SqlType; 3] = [SqlType::Text, SqlType::Bytes, SqlType::Text];

/// Column types of a running attempt row: id, version, checksum, policy.
pub const RUNNING_COLUMNS: [SqlType; 4] =
    [SqlType::I64, SqlType::Text, SqlType::Bytes, SqlType::Text];

/// The statuses of an `up` journal row, as `sql/<engine>.sql` documents them.
pub mod status {
    /// Journaled before the body ran, still unresolved.
    pub const RUNNING: &str = "running";
    /// The body completed and the runner recorded it.
    pub const APPLIED: &str = "applied";
    /// The body failed and rolled back, or never started.
    pub const FAILED: &str = "failed";
    /// An interrupted transactional attempt, found later under exclusion.
    pub const ROLLED_BACK: &str = "rolled_back";
    /// An operator asserted the effects complete.
    pub const RECOVERED: &str = "recovered";
    /// An operator asserted the effects undone.
    pub const ABANDONED: &str = "abandoned";
}

/// The `policy` an attempt journals: a running transactional attempt has
/// provably not committed once its session ended.
pub fn policy(policy: Policy) -> Value {
    Value::Text(policy.as_str().to_owned())
}

/// Refuses a recovery assertion without a reason, before the target is touched.
pub fn check_reason<E: std::error::Error>(recovery: &Recovery<'_>) -> Result<(), Error<E>> {
    if recovery.reason.trim().is_empty() {
        return Err(RecoveryError::MissingReason.into());
    }

    Ok(())
}

/// Checks an assertion about `attempt` against its journal row, read under
/// exclusion, and the catalog's `entries`, returning the status it records. A
/// completed artifact must be the one the attempt ran; a restored one need
/// not be in the catalog any more.
pub fn resolve<E: std::error::Error>(
    attempt: i64,
    action: RecoveryAction,
    rows: Vec<Vec<Value>>,
    entries: &[CatalogEntry],
) -> Result<&'static str, Error<E>> {
    let Some(row) = rows.into_iter().next() else {
        return Err(RecoveryError::UnknownAttempt(attempt).into());
    };
    let Ok(
        [
            Value::Text(version),
            Value::Bytes(checksum),
            Value::Text(current),
        ],
    ) = <[Value; 3]>::try_from(row)
    else {
        return Err(Error::History("invalid attempt row".into()));
    };

    if current != status::RUNNING {
        return Err(RecoveryError::Resolved(attempt).into());
    }

    if action != RecoveryAction::Completed(attempt) {
        return Ok(status::ABANDONED);
    }

    let Some(entry) = entries.iter().find(|e| e.version.to_string() == version) else {
        return Err(RecoveryError::Missing { attempt, version }.into());
    };

    if entry.checksum.as_bytes().as_slice() != checksum.as_slice() {
        return Err(RecoveryError::Changed { attempt, version }.into());
    }

    Ok(status::RECOVERED)
}

/// The parameters that finish running attempt `attempt` with `status`: the
/// id, the status, and an operator's reason and evidence, if recovered.
pub fn finish(attempt: i64, status: &str, recovery: Option<&Recovery<'_>>) -> [Value; 4] {
    let text = |value: Option<&str>| {
        value.map_or(Value::Null(SqlType::Text), |value| {
            Value::Text(value.to_owned())
        })
    };

    [
        Value::I64(attempt),
        Value::Text(status.to_owned()),
        text(recovery.map(|r| r.reason)),
        text(recovery.and_then(|r| r.evidence)),
    ]
}

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
            let [
                rank,
                Value::Text(version),
                Value::Bytes(checksum),
                Value::Text(recorded),
            ] = <[Value; 4]>::try_from(row)
                .map_err(|_| Error::History("invalid history row".into()))?
            else {
                return Err(Error::History("invalid history row".into()));
            };
            let provenance = match recorded.as_str() {
                status::APPLIED => Provenance::Executed,
                status::RECOVERED => Provenance::Recovered,
                _ => return Err(Error::History("invalid history status".into())),
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
                provenance,
            })
        })
        .collect()
}

/// A target without a journal: fresh without user objects, unmanaged with
/// `objects` of them.
pub fn absent(objects: &Value) -> Journal {
    if int(objects) == Some(0) {
        Journal::Fresh
    } else {
        Journal::Unmanaged
    }
}

/// Decodes a journal's records: its stored `stream` and `scheme`, history
/// rows and running attempt rows.
pub fn records<E: std::error::Error>(
    stream: Value,
    scheme: Value,
    history: Vec<Vec<Value>>,
    running: Vec<Vec<Value>>,
) -> Result<Journal, Error<E>> {
    let (Some(stream), Some(scheme)) = (text(stream), text(scheme)) else {
        return Err(Error::History("invalid journal header".into()));
    };

    Ok(Journal::Managed(Records {
        stream,
        scheme,
        history: decode_history(history)?,
        attempts: decode_attempts(running)?,
    }))
}

/// Decodes running attempt rows.
fn decode_attempts<E: std::error::Error>(rows: Vec<Vec<Value>>) -> Result<Vec<Attempt>, Error<E>> {
    rows.into_iter()
        .map(|row| {
            let invalid = || Error::History("invalid attempt row".into());
            let [
                id,
                Value::Text(version),
                Value::Bytes(checksum),
                Value::Text(policy),
            ] = <[Value; 4]>::try_from(row).map_err(|_| invalid())?
            else {
                return Err(invalid());
            };
            let policy = [Policy::TransactionalRequired, Policy::NonTransactional]
                .into_iter()
                .find(|known| known.as_str() == policy)
                .ok_or_else(invalid)?;
            let checksum = checksum.try_into().map_err(|_| invalid())?;

            Ok(Attempt {
                id: int(&id).ok_or_else(invalid)?,
                version,
                checksum: Checksum::from_bytes(checksum),
                policy,
            })
        })
        .collect()
}
