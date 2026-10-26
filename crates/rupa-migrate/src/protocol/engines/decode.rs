//! Decoding of the journal rows the engines read.

use rupa_core::Value;

use super::{int, status, text};
use crate::{
    Attempt, Checksum, Coverage, CoverageMember, CoverageSource, Direction, Error, HistoryEntry,
    Journal, MigrationKind, Policy, Provenance, Records,
};

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

            Ok(HistoryEntry {
                installation_rank: rank_of(&rank)?,
                version,
                checksum: checksum_of(checksum)?,
                provenance: provenance_of(&recorded)?,
            })
        })
        .collect()
}

/// A positive installation rank.
fn rank_of<E: std::error::Error>(rank: &Value) -> Result<u64, Error<E>> {
    int(rank)
        .and_then(|rank| u64::try_from(rank).ok())
        .filter(|rank| *rank > 0)
        .ok_or_else(|| Error::History("invalid installation rank".into()))
}

/// A stored checksum.
fn checksum_of<E: std::error::Error>(checksum: Vec<u8>) -> Result<Checksum, Error<E>> {
    let bytes = checksum
        .try_into()
        .map_err(|_| Error::History("invalid stored checksum length".into()))?;

    Ok(Checksum::from_bytes(bytes))
}

/// The provenance of a completed status.
fn provenance_of<E: std::error::Error>(recorded: &str) -> Result<Provenance, Error<E>> {
    match recorded {
        status::APPLIED => Ok(Provenance::Executed),
        status::RECOVERED => Ok(Provenance::Recovered),
        _ => Err(Error::History("invalid history status".into())),
    }
}

/// Decodes coverage rows, which name their snapshot or adoption source. A
/// target has at most one source.
pub fn decode_coverage<E: std::error::Error>(
    rows: Vec<Vec<Value>>,
) -> Result<Option<Coverage>, Error<E>> {
    let mut coverage: Option<Coverage> = None;

    for row in rows {
        let invalid = || Error::History("invalid coverage row".into());
        let [
            Value::Text(kind),
            id,
            Value::Text(boundary),
            checksum,
            recorded,
            Value::Text(version),
            Value::Bytes(member),
        ] = <[Value; 7]>::try_from(row).map_err(|_| invalid())?
        else {
            return Err(invalid());
        };
        let source = match (kind.as_str(), checksum) {
            ("snapshot", Value::Bytes(checksum)) => CoverageSource::Snapshot {
                installation_rank: rank_of(&id)?,
                checksum: checksum_of(checksum)?,
                provenance: provenance_of(&text(recorded).unwrap_or_default())?,
            },
            ("adopt", _) => CoverageSource::Adoption {
                id: int(&id).ok_or_else(invalid)?,
            },
            _ => return Err(invalid()),
        };
        let member = CoverageMember {
            version,
            checksum: checksum_of(member)?,
        };

        match &mut coverage {
            Some(coverage) if coverage.source == source => coverage.members.push(member),
            Some(_) => return Err(Error::History("more than one coverage claim".into())),
            None => {
                coverage = Some(Coverage {
                    boundary,
                    source,
                    members: vec![member],
                })
            }
        }
    }

    Ok(coverage)
}

/// The rows a journal's records decode from.
pub struct Rows {
    /// The stored stream.
    pub stream: Value,
    /// The stored version scheme.
    pub scheme: Value,
    /// History rows.
    pub history: Vec<Vec<Value>>,
    /// Coverage rows.
    pub coverage: Vec<Vec<Value>>,
    /// Running attempt rows.
    pub running: Vec<Vec<Value>>,
}

/// Decodes a journal's records.
pub fn records<E: std::error::Error>(rows: Rows) -> Result<Journal, Error<E>> {
    let (Some(stream), Some(scheme)) = (text(rows.stream), text(rows.scheme)) else {
        return Err(Error::History("invalid journal header".into()));
    };

    Ok(Journal::Managed(Records {
        stream,
        scheme,
        history: decode_history(rows.history)?,
        coverage: decode_coverage(rows.coverage)?,
        attempts: decode_attempts(rows.running)?,
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
                Value::Text(kind),
                undoes,
            ] = <[Value; 6]>::try_from(row).map_err(|_| invalid())?
            else {
                return Err(invalid());
            };
            let policy = [Policy::TransactionalRequired, Policy::NonTransactional]
                .into_iter()
                .find(|known| known.as_str() == policy)
                .ok_or_else(invalid)?;
            let checksum = checksum.try_into().map_err(|_| invalid())?;
            let undoes = int(&undoes).and_then(|rank| u64::try_from(rank).ok());
            let (kind, direction) = match (kind.as_str(), undoes) {
                ("up", None) => (MigrationKind::Incremental, Direction::Up),
                ("snapshot", None) => (MigrationKind::Snapshot, Direction::Up),
                ("down", Some(undoes)) => (MigrationKind::Incremental, Direction::Down { undoes }),
                _ => return Err(invalid()),
            };

            Ok(Attempt {
                id: int(&id).ok_or_else(invalid)?,
                version,
                checksum: Checksum::from_bytes(checksum),
                policy,
                kind,
                direction,
            })
        })
        .collect()
}
