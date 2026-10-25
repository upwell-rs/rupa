//! Read-only inspection: what a target's journal records, classified against
//! a catalog, and what an apply would run. Nothing here acquires exclusion,
//! writes or runs a body.

use std::collections::BTreeSet;

use rupa_core::DialectId;

use crate::{Catalog, CatalogEntry, Checksum, HistoryEntry, Policy, ValidationError, Version};

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
    /// An installed version the catalog does not contain.
    Missing {
        /// The installed version, as stored.
        version: String,
        /// The installed checksum.
        checksum: Checksum,
    },
    /// An installed version whose catalog artifact changed.
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
    /// A pending version below the newest installed one, which strict apply refuses.
    Late {
        /// The version.
        version: Version,
        /// The catalog's checksum.
        checksum: Checksum,
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
    /// Running attempts, oldest first.
    pub attempts: Vec<Attempt>,
    /// Catalog entries not installed, in version order; empty when the
    /// journal belongs to another stream.
    pub pending: Vec<&'c CatalogEntry>,
    /// Everything that stops apply, each with its artifact identity. Apply
    /// runs `pending` only when this is empty.
    pub blockers: Vec<Blocker>,
}

/// What apply would do on a target: its status and the exact steps it
/// would run, if nothing blocks it.
#[derive(Debug, Clone)]
pub struct ApplyPlan<'c> {
    /// The target's status; apply runs `steps` only when its blockers are empty.
    pub status: Status<'c>,
    /// The pending entries, in the order apply runs them.
    pub steps: Vec<Step<'c>>,
}

/// One pending entry apply would run: its identity, policy and body.
#[derive(Debug, Clone)]
pub struct Step<'c> {
    /// The entry: version, checksum and metadata, policy included.
    pub entry: &'c CatalogEntry,
    /// What its body sends.
    pub execution: Execution,
}

/// What a pending entry's body sends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Execution {
    /// A planned body: one script per plan operation, exactly as apply sends
    /// them for the session's dialect.
    Static(Vec<String>),
    /// An imperative Rust body, opaque until it runs.
    Opaque,
}

impl<M> Catalog<'_, M> {
    /// Classifies `inspection` against this catalog. Pure: performs no IO.
    pub fn classify(&self, inspection: Inspection) -> Status<'_> {
        let Inspection { exclusion, journal } = inspection;
        let mut status = Status {
            classification: Classification::Fresh,
            exclusion,
            history: Vec::new(),
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
                status.pending = self.entries.iter().collect();

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
            let installed = self.compare(&records.history, &mut status.blockers);

            status.pending = self
                .entries
                .iter()
                .filter(|entry| !installed.contains(&entry.version))
                .collect();
        }

        status.history = records.history;
        status.attempts = records.attempts;

        status
    }

    /// The installed versions the catalog contains, adding a blocker for
    /// every installed version that is missing, changed or recorded twice,
    /// and for every pending one below the newest installed version.
    fn compare(&self, history: &[HistoryEntry], blockers: &mut Vec<Blocker>) -> BTreeSet<Version> {
        let mut installed = BTreeSet::new();
        let mut newest = None;

        for record in history {
            let version = self.scheme.parse(&record.version).ok();
            let entry = version
                .and_then(|version| self.entries.iter().find(|entry| entry.version == version));

            newest = newest.max(version);

            let Some(entry) = entry else {
                blockers.push(Blocker::Missing {
                    version: record.version.clone(),
                    checksum: record.checksum,
                });

                continue;
            };

            if !installed.insert(entry.version) {
                blockers.push(Blocker::Duplicate(entry.version));
            }

            if entry.checksum != record.checksum {
                blockers.push(Blocker::Changed {
                    version: entry.version,
                    installed: record.checksum,
                    catalog: entry.checksum,
                });
            }
        }

        let late = self
            .entries
            .iter()
            .filter(|entry| newest.is_some_and(|newest| entry.version < newest))
            .filter(|entry| !installed.contains(&entry.version))
            .map(|entry| Blocker::Late {
                version: entry.version,
                checksum: entry.checksum,
            });

        blockers.extend(late);

        installed
    }

    /// The steps apply would run for `status`, rendered for `dialect`.
    pub fn plan_for<'c>(
        &self,
        status: Status<'c>,
        dialect: DialectId,
    ) -> Result<ApplyPlan<'c>, ValidationError> {
        let steps = status
            .pending
            .iter()
            .map(|entry| {
                let execution = match &entry.plan {
                    Some(plan) => Execution::Static(
                        plan.operations()
                            .iter()
                            .map(|operation| crate::catalog::render(operation, dialect))
                            .collect::<Result<_, _>>()?,
                    ),
                    None => Execution::Opaque,
                };

                Ok(Step { entry, execution })
            })
            .collect::<Result<_, ValidationError>>()?;

        Ok(ApplyPlan { status, steps })
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
