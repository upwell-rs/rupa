//! The checks a run makes under exclusion before it runs anything.

use crate::status::{Schedule, Selection};
use crate::{
    AdoptionError, ApplyOptions, Catalog, Coverage, Covered, Error, HistoryEntry, Order,
    RevertTarget,
};

/// What an apply with `options` runs on a target with `history` and
/// `coverage`, after checking them against the catalog with the comparison
/// status uses: every installed or covered artifact is present, unchanged
/// and recorded once, nothing pending is uncovered, and nothing pending is
/// late unless `options` permit it.
pub(crate) fn schedule<'c, E: std::error::Error, M>(
    catalog: &'c Catalog<'_, M>,
    history: &[HistoryEntry],
    coverage: Option<&Coverage>,
    options: &ApplyOptions,
) -> Result<Schedule<'c>, Error<E>> {
    let mut blockers = Vec::new();
    let installed = catalog.compare(history, coverage, &mut blockers);
    let pending: Vec<_> = catalog
        .incrementals()
        .filter(|entry| !installed.contains(&entry.version))
        .collect();
    let initial = history.is_empty() && coverage.is_none();
    let schedule = catalog.schedule(&pending, initial, options, &mut blockers);

    if !blockers.is_empty() {
        return Err(Error::Blocked(blockers));
    }

    Ok(schedule)
}

/// The installations a revert to `target` undoes on a target with
/// `history` and `coverage`, after checking them against the catalog as
/// apply does, except that late artifacts never block a revert.
pub(crate) fn undo<'c, E: std::error::Error, M>(
    catalog: &'c Catalog<'_, M>,
    history: &[HistoryEntry],
    coverage: Option<&Coverage>,
    target: RevertTarget,
) -> Result<Selection<'c>, Error<E>> {
    let mut blockers = Vec::new();
    let boundary = catalog.boundary(coverage);

    catalog.compare(history, coverage, &mut blockers);
    Order::AllowLate.permit(&mut blockers);

    let selection = catalog.select(history, boundary, target, &mut blockers);

    if !blockers.is_empty() {
        return Err(Error::Blocked(blockers));
    }

    Ok(selection)
}

/// The incrementals an adoption through `through` covers, after checking
/// that it has a reason and covers something.
pub(crate) fn adopted<E: std::error::Error, M>(
    catalog: &Catalog<'_, M>,
    adoption: &crate::Adoption<'_>,
) -> Result<Vec<Covered>, Error<E>> {
    let members: Vec<_> = catalog
        .incrementals()
        .filter(|entry| entry.version <= adoption.through)
        .map(|entry| Covered {
            version: entry.version,
            checksum: entry.checksum,
        })
        .collect();

    if adoption.reason.trim().is_empty() {
        return Err(AdoptionError::MissingReason.into());
    }

    if members.is_empty() {
        return Err(AdoptionError::NothingCovered(adoption.through).into());
    }

    Ok(members)
}
