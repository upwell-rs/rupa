use rupa_core::DialectId;

use super::{Blocker, Execution, Status};
use crate::{
    Catalog, CatalogEntry, HistoryEntry, Order, Reversal, RevertTarget, ValidationError, Version,
};

/// What a revert would do on a target: its status and the installations it
/// would undo, if nothing blocks it.
#[derive(Debug, Clone)]
pub struct RevertPlan<'c> {
    /// The target's status. Its blockers are those that stop a revert: late
    /// artifacts never do, and the revert's own refusals are added. A revert
    /// runs `steps` only when they are empty.
    pub status: Status<'c>,
    /// The reversible installations to undo, in the order a revert runs
    /// them: newest installation first.
    pub steps: Vec<Undo<'c>>,
    /// The versions that stay installed, in installation order.
    pub remaining: Vec<Version>,
}

/// One installation a revert would undo: its rank, entry and down body.
#[derive(Debug, Clone)]
pub struct Undo<'c> {
    /// The installation rank it undoes.
    pub installation_rank: u64,
    /// The entry: version, checksum and metadata, policy included.
    pub entry: &'c CatalogEntry,
    /// What its down body sends.
    pub execution: Execution,
}

/// The installations a revert undoes and the versions it keeps.
#[derive(Debug, Default)]
pub(crate) struct Selection<'c> {
    /// Rank and entry of each reversible installation to undo, newest first.
    pub(crate) undo: Vec<(u64, &'c CatalogEntry)>,
    /// The versions kept, in installation order.
    pub(crate) remaining: Vec<Version>,
}

impl<M> Catalog<'_, M> {
    /// The steps a revert to `target` would run for `status`, rendered for
    /// `dialect`.
    pub fn revert_for<'c>(
        &'c self,
        mut status: Status<'c>,
        dialect: DialectId,
        target: RevertTarget,
    ) -> Result<RevertPlan<'c>, ValidationError> {
        let boundary = self.boundary(status.coverage.as_ref());

        Order::AllowLate.permit(&mut status.blockers);

        let Selection { undo, remaining } =
            self.select(&status.history, boundary, target, &mut status.blockers);
        let steps = undo
            .into_iter()
            .map(|(installation_rank, entry)| {
                let execution = Execution::of(entry.down(), dialect)?;

                Ok(Undo {
                    installation_rank,
                    entry,
                    execution,
                })
            })
            .collect::<Result<_, ValidationError>>()?;

        Ok(RevertPlan {
            status,
            steps,
            remaining,
        })
    }

    /// Splits `history`, in installation order, into the newest installations
    /// `target` undoes and the rest, adding a blocker for every undone
    /// installation that is irreversible or that the target retains, for a
    /// count beyond the installations, and for a target version below the
    /// coverage `boundary`. An installation the catalog lacks is skipped:
    /// comparing history reports it.
    pub(crate) fn select(
        &self,
        history: &[HistoryEntry],
        boundary: Option<Version>,
        target: RevertTarget,
        blockers: &mut Vec<Blocker>,
    ) -> Selection<'_> {
        let installed: Vec<_> = history
            .iter()
            .map(|record| {
                let version = self.scheme.parse(&record.version).ok();

                (record.installation_rank, version)
            })
            .collect();
        let count = match target {
            RevertTarget::Last(count) => count,
            RevertTarget::To(target) => installed
                .iter()
                .filter(|(_, version)| version.is_some_and(|version| version > target))
                .count(),
        };
        let split = installed.len().checked_sub(count);
        let (kept, undone) = installed.split_at(split.unwrap_or(installed.len()));
        let mut selection = Selection::default();

        if let (RevertTarget::To(requested), Some(boundary)) = (target, boundary)
            && requested < boundary
        {
            blockers.push(Blocker::Boundary {
                requested,
                boundary,
            });
        }

        if split.is_none() {
            blockers.push(Blocker::Exceeds {
                requested: count,
                installed: installed.len(),
            });
        }

        for (rank, version) in undone.iter().rev() {
            let Some(version) = *version else {
                continue;
            };

            if matches!(target, RevertTarget::To(target) if version <= target) {
                blockers.push(Blocker::Interleaved(version));

                continue;
            }

            let Some(entry) = self.incrementals().find(|entry| entry.version == version) else {
                continue;
            };

            if entry.reversal == Reversal::Irreversible {
                blockers.push(Blocker::Irreversible(version));
            } else {
                selection.undo.push((*rank, entry));
            }
        }

        selection.remaining = kept.iter().filter_map(|(_, version)| *version).collect();

        selection
    }
}
