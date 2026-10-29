//! The engine protocols and the row helpers their steps share; they exist
//! only with a mode to generate steps for.

mod decode;
pub mod mysql;
pub mod postgres;
pub mod sqlite;

pub use decode::*;

use rupa_core::{SqlType, Value};

use crate::{
    Adoption, AdoptionError, CatalogEntry, Covered, Direction, Error, Journal, MigrationKind,
    Policy, Recovery, RecoveryAction, RecoveryError,
};

/// Column types of a history row: rank, version, checksum, status.
pub const HISTORY_COLUMNS: [SqlType; 4] =
    [SqlType::I64, SqlType::Text, SqlType::Bytes, SqlType::Text];

/// Column types of an attempt row: version, checksum, status, kind.
pub const ATTEMPT_COLUMNS: [SqlType; 4] =
    [SqlType::Text, SqlType::Bytes, SqlType::Text, SqlType::Text];

/// Column types of a coverage row: the source's kind, id, version,
/// checksum and status, then the member's version and checksum.
pub const COVERAGE_COLUMNS: [SqlType; 7] = [
    SqlType::Text,
    SqlType::I64,
    SqlType::Text,
    SqlType::Bytes,
    SqlType::Text,
    SqlType::Text,
    SqlType::Bytes,
];

/// Column types of a running attempt row: id, version, checksum, policy,
/// kind, undone installation.
pub const RUNNING_COLUMNS: [SqlType; 6] = [
    SqlType::I64,
    SqlType::Text,
    SqlType::Bytes,
    SqlType::Text,
    SqlType::Text,
    SqlType::I64,
];

/// The statuses of an `up` or `down` journal row, as `sql/<engine>.sql`
/// documents them.
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

/// The parameters that journal a running attempt of `entry` in `direction`:
/// kind, version, checksum, policy and the installation it undoes.
pub fn attempt_row(entry: &CatalogEntry, direction: Direction) -> [Value; 5] {
    let (kind, undoes) = match (entry.metadata.kind, direction) {
        (MigrationKind::Snapshot, Direction::Up) => ("snapshot", Value::Null(SqlType::I64)),
        (MigrationKind::Incremental, Direction::Up) => ("up", Value::Null(SqlType::I64)),
        (_, Direction::Down { undoes }) => ("down", Value::I64(undoes.cast_signed())),
    };

    [
        Value::Text(kind.to_owned()),
        Value::Text(entry.version.to_string()),
        Value::Bytes(entry.checksum.as_bytes().to_vec()),
        policy(entry.metadata.policy),
        undoes,
    ]
}

/// The parameters that record `member` covered by the snapshot attempt or
/// adoption event `source`: version, checksum, source.
pub fn covered_row(member: &Covered, source: i64) -> [Value; 3] {
    [
        Value::Text(member.version.to_string()),
        Value::Bytes(member.checksum.as_bytes().to_vec()),
        Value::I64(source),
    ]
}

/// The parameters that record an adoption event: boundary, reason, evidence.
pub fn adoption_row(adoption: &Adoption<'_>) -> [Value; 3] {
    [
        Value::Text(adoption.through.to_string()),
        Value::Text(adoption.reason.to_owned()),
        adoption
            .evidence
            .map_or(Value::Null(SqlType::Text), |evidence| {
                Value::Text(evidence.to_owned())
            }),
    ]
}

/// Refuses to adopt a target without a journal and without user objects,
/// counted as `objects`: apply creates a fresh target's schema.
pub fn check_unmanaged<E: std::error::Error>(objects: &Value) -> Result<(), Error<E>> {
    if int(objects) == Some(0) {
        return Err(AdoptionError::Fresh.into());
    }

    Ok(())
}

/// Refuses a recovery assertion without a reason, before the target is touched.
pub fn check_reason<E: std::error::Error>(recovery: &Recovery<'_>) -> Result<(), Error<E>> {
    if recovery.reason.trim().is_empty() {
        return Err(RecoveryError::MissingReason.into());
    }

    Ok(())
}

/// Checks an assertion about `attempt` against its journal row, read under
/// exclusion, and the catalog's `entries`, returning the status it records
/// and the incrementals a completed snapshot covers, which the engine
/// records with it. A completed artifact must be the one the attempt ran; a
/// restored one need not be in the catalog any more.
pub fn resolve<E: std::error::Error>(
    attempt: i64,
    action: RecoveryAction,
    rows: Vec<Vec<Value>>,
    entries: &[CatalogEntry],
) -> Result<(&'static str, &[Covered]), Error<E>> {
    let Some(row) = rows.into_iter().next() else {
        return Err(RecoveryError::UnknownAttempt(attempt).into());
    };
    let Ok(
        [
            Value::Text(version),
            Value::Bytes(checksum),
            Value::Text(current),
            Value::Text(kind),
        ],
    ) = <[Value; 4]>::try_from(row)
    else {
        return Err(Error::History("invalid attempt row".into()));
    };
    let kind = match kind.as_str() {
        "snapshot" => MigrationKind::Snapshot,
        _ => MigrationKind::Incremental,
    };

    if current != status::RUNNING {
        return Err(RecoveryError::Resolved(attempt).into());
    }

    if action != RecoveryAction::Completed(attempt) {
        return Ok((status::ABANDONED, &[]));
    }

    let Some(entry) = entries
        .iter()
        .find(|e| e.metadata.kind == kind && e.version.to_string() == version)
    else {
        return Err(RecoveryError::Missing { attempt, version }.into());
    };

    if entry.checksum.as_bytes().as_slice() != checksum.as_slice() {
        return Err(RecoveryError::Changed { attempt, version }.into());
    }

    Ok((status::RECOVERED, &entry.covers))
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

/// A target without a journal: fresh without user objects, unmanaged with
/// `objects` of them.
pub fn absent(objects: &Value) -> Journal {
    if int(objects) == Some(0) {
        Journal::Fresh
    } else {
        Journal::Unmanaged
    }
}
