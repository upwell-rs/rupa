//! Read-only inspection: what a target's journal records, classified against
//! a catalog, and what an apply would run. Nothing here acquires exclusion,
//! writes or runs a body.

mod compare;
mod plan;
mod revert;

use crate::{
    Catalog, CatalogEntry, Checksum, Coverage, HistoryEntry, MigrationKind, Policy, Version,
};

#[cfg(any(feature = "sync", feature = "async"))]
pub(crate) use plan::Schedule;
pub use plan::{ApplyPlan, Execution, Step};
#[cfg(any(feature = "sync", feature = "async"))]
pub(crate) use revert::Selection;
pub use revert::{RevertPlan, Undo};

/// What a session read from a target, without exclusion or writes.
#[derive(Debug, Clone)]
pub struct Inspection {
    /// Whether another runner excludes the target now.
    pub exclusion: Exclusion,
    /// The target's journal, if it has one.
    pub journal: Journal,
}

/// Whether a runner excludes the target at the moment it was read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exclusion {
    /// No runner holds exclusion.
    Free,
    /// Another session holds the target's PostgreSQL advisory lock or MySQL
    /// named lock, so a runner is live.
    Locked,
    /// SQLite's durable owner claim is set. Its runner may be live or may
    /// have stopped; only an operator can tell, and release it with
    /// [`RecoveryAction::ReleaseClaim`](crate::RecoveryAction::ReleaseClaim).
    Claimed,
}

/// A target's journal, as read.
#[derive(Debug, Clone)]
pub enum Journal {
    /// No journal and no user objects: apply would create the journal.
    Fresh,
    /// No journal, but user objects: apply refuses the target until it is adopted.
    Unmanaged,
    /// A valid journal and what it records.
    Managed(Records),
}

/// What a valid journal records.
#[derive(Debug, Clone)]
pub struct Records {
    /// The stream the journal belongs to.
    pub stream: String,
    /// The version scheme of its history.
    pub scheme: String,
    /// Installed history, in installation order.
    pub history: Vec<HistoryEntry>,
    /// The coverage claim of an executed snapshot or an adoption, if any.
    pub coverage: Option<Coverage>,
    /// Whether the journal records a completed installation or coverage
    /// claim, even one since undone. Until it does, apply may initialize the
    /// target from a snapshot; a journal of failed or rolled-back attempts
    /// only has not been initialized.
    pub initialized: bool,
    /// Attempts journaled as running, oldest first: interrupted, unresolved
    /// or, while a runner is live, in progress.
    pub attempts: Vec<Attempt>,
}

/// A journaled attempt that is still running.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attempt {
    /// The attempt id, which recovery names.
    pub id: i64,
    /// The version it runs, as stored.
    pub version: String,
    /// The checksum it runs.
    pub checksum: Checksum,
    /// The policy it runs under.
    pub policy: Policy,
    /// Whether it runs an incremental or a snapshot.
    pub kind: MigrationKind,
    /// Whether it installs its artifact or undoes an installation.
    pub direction: Direction,
}

/// Whether an attempt installs its artifact or undoes an installation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Runs the artifact's up body; completion installs it with a new rank.
    Up,
    /// Runs the artifact's down body; completion makes installation
    /// `undoes` inactive, keeping its journal row.
    Down {
        /// The installation rank it undoes.
        undoes: u64,
    },
}

/// What a target is, against a catalog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Classification {
    /// No journal and no user objects.
    Fresh,
    /// A journal with no running attempts and no owner claim.
    Managed,
    /// No journal, but user objects.
    Unmanaged,
    /// A journal with running attempts or an owner claim, whose outcome only
    /// the next apply or an operator can settle.
    Uncertain,
}

/// A condition that stops apply before it runs anything, with the identity
/// of the artifact or attempt it concerns.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum Blocker {
    /// The target has user objects but no journal: it needs adoption.
    Unmanaged,
    /// The journal belongs to another stream or version scheme; history is
    /// not compared against this catalog.
    Stream {
        /// The stored stream.
        stream: String,
        /// The stored version scheme.
        scheme: String,
    },
    /// Another runner holds exclusion; apply would report
    /// [`Error::Busy`](crate::Error::Busy).
    Locked,
    /// SQLite's owner claim is set; apply would report
    /// [`Error::Busy`](crate::Error::Busy) until the claim is released.
    Claimed,
    /// An interrupted transactional attempt, which provably never committed:
    /// apply records it rolled back and stops with
    /// [`Error::Reconciled`](crate::Error::Reconciled).
    Interrupted(Attempt),
    /// An unresolved nontransactional attempt, whose effects may persist:
    /// apply stops with [`Error::Uncertain`](crate::Error::Uncertain) until
    /// an operator recovers it.
    Unresolved(Attempt),
    /// An installed or covered version, or an executed snapshot, that the
    /// catalog does not contain.
    Missing {
        /// The installed version, as stored.
        version: String,
        /// The installed checksum.
        checksum: Checksum,
    },
    /// An installed or covered version, or an executed snapshot, whose
    /// catalog artifact changed. A snapshot's checksum covers its coverage
    /// manifest, so changing or adding an incremental it covers changes it.
    Changed {
        /// The version.
        version: Version,
        /// The checksum history recorded.
        installed: Checksum,
        /// The catalog's checksum.
        catalog: Checksum,
    },
    /// A version history records more than once.
    Duplicate(Version),
    /// A pending version below the newest installed one, which strict apply
    /// refuses. Never blocks a revert.
    Late {
        /// The version.
        version: Version,
        /// The catalog's checksum.
        checksum: Checksum,
    },
    /// A catalog incremental at or below the target's coverage boundary
    /// that the coverage claim does not list: it was added after the
    /// snapshot ran or the target was adopted. No order permits it; write a
    /// corrective migration above the boundary instead.
    Uncovered {
        /// The version.
        version: Version,
        /// The coverage boundary.
        boundary: Version,
    },
    /// An installation a revert would undo has no down body. Blocks only a revert.
    Irreversible(Version),
    /// A retained installation sits among those a revert to a version would
    /// undo, so they are not the newest installations. Undoing them would
    /// leave it running on top of reverted state. Blocks only a revert.
    Interleaved(Version),
    /// A revert of the last installations asks for more than are installed.
    /// Blocks only a revert.
    Exceeds {
        /// The installations requested.
        requested: usize,
        /// The installations present.
        installed: usize,
    },
    /// A revert to a version below the coverage boundary. Covered
    /// incrementals never ran individually, so no down body undoes them.
    /// Blocks only a revert.
    Boundary {
        /// The requested version.
        requested: Version,
        /// The coverage boundary.
        boundary: Version,
    },
}

/// A catalog against a target, read without exclusion, writes or bodies.
#[derive(Debug, Clone)]
pub struct Status<'c> {
    /// What the target is.
    pub classification: Classification,
    /// Whether a runner excluded the target when it was read.
    pub exclusion: Exclusion,
    /// Installed history, in installation order, with provenance.
    pub history: Vec<HistoryEntry>,
    /// The coverage claim of an executed snapshot or an adoption, if any.
    pub coverage: Option<Coverage>,
    /// Whether the journal records a completed installation or coverage
    /// claim, even one since undone. A target that was never initialized may
    /// start from a snapshot; reverting every installation does not make a
    /// target uninitialized again.
    pub initialized: bool,
    /// Running attempts, oldest first.
    pub attempts: Vec<Attempt>,
    /// Catalog incrementals neither installed nor covered, in version
    /// order; empty when the journal belongs to another stream.
    pub pending: Vec<&'c CatalogEntry>,
    /// Everything that stops apply, each with its artifact identity. Apply
    /// runs `pending` only when this is empty.
    pub blockers: Vec<Blocker>,
}

impl<M> Catalog<'_, M> {
    /// Classifies `inspection` against this catalog. Pure: performs no IO.
    pub fn classify(&self, inspection: Inspection) -> Status<'_> {
        let Inspection { exclusion, journal } = inspection;
        let mut status = Status {
            classification: Classification::Fresh,
            exclusion,
            history: Vec::new(),
            coverage: None,
            initialized: false,
            attempts: Vec::new(),
            pending: Vec::new(),
            blockers: match exclusion {
                Exclusion::Free => Vec::new(),
                Exclusion::Locked => vec![Blocker::Locked],
                Exclusion::Claimed => vec![Blocker::Claimed],
            },
        };

        let records = match journal {
            Journal::Fresh => {
                status.pending = self.incrementals().collect();

                return status;
            }
            Journal::Unmanaged => {
                status.classification = Classification::Unmanaged;
                status.blockers.push(Blocker::Unmanaged);

                return status;
            }
            Journal::Managed(records) => records,
        };

        status.classification = if records.attempts.is_empty() && exclusion != Exclusion::Claimed {
            Classification::Managed
        } else {
            Classification::Uncertain
        };

        if exclusion == Exclusion::Free {
            status.blockers.extend(records.attempts.iter().map(blocker));
        }

        if records.stream != self.stream || records.scheme != self.scheme.as_str() {
            status.blockers.push(Blocker::Stream {
                stream: records.stream,
                scheme: records.scheme,
            });
        } else {
            let installed = self.compare(
                &records.history,
                records.coverage.as_ref(),
                &mut status.blockers,
            );

            status.pending = self
                .incrementals()
                .filter(|entry| !installed.contains(&entry.version))
                .collect();
        }

        status.history = records.history;
        status.coverage = records.coverage;
        status.initialized = records.initialized;
        status.attempts = records.attempts;

        status
    }
}

/// The blocker a running attempt is when no runner is live.
fn blocker(attempt: &Attempt) -> Blocker {
    match attempt.policy {
        Policy::TransactionalRequired => Blocker::Interrupted(attempt.clone()),
        Policy::NonTransactional => Blocker::Unresolved(attempt.clone()),
    }
}

#[cfg(test)]
mod tests;
