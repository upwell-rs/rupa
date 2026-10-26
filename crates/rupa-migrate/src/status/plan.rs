use rupa_core::DialectId;

use super::{Blocker, Status};
use crate::{ApplyOptions, Catalog, CatalogEntry, Init, Plan, ValidationError, Version};

/// What apply would do on a target: its status and the exact steps it
/// would run, if nothing blocks it.
#[derive(Debug, Clone)]
pub struct ApplyPlan<'c> {
    /// The target's status; apply runs `steps` only when its blockers are
    /// empty. Late blockers the options permit are moved to `late`.
    pub status: Status<'c>,
    /// The entries apply would run, in order: the snapshot that
    /// initializes a fresh target, if one is selected, then the pending
    /// incrementals.
    pub steps: Vec<Step<'c>>,
    /// The incrementals the selected snapshot covers, in version order;
    /// apply records them without running their bodies.
    pub covered: Vec<Version>,
    /// The pending versions below an installed version that the options
    /// permit; apply runs them with the other steps and reports them late.
    pub late: Vec<Version>,
}

/// What an apply runs, in order.
pub(crate) struct Schedule<'c> {
    /// The snapshot that initializes a fresh target, if one is selected.
    pub(crate) snapshot: Option<&'c CatalogEntry>,
    /// The incrementals to run after it, in version order.
    pub(crate) entries: Vec<&'c CatalogEntry>,
    /// Those below an installed version, which the options permit.
    pub(crate) late: Vec<Version>,
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

impl Execution {
    /// What a body sends: `plan`'s operations rendered for `dialect`, or
    /// opaque without a plan.
    pub(crate) fn of(plan: Option<&Plan>, dialect: DialectId) -> Result<Self, ValidationError> {
        let Some(plan) = plan else {
            return Ok(Self::Opaque);
        };
        let scripts = plan
            .operations()
            .iter()
            .map(|operation| crate::catalog::render(operation, dialect))
            .collect::<Result<_, _>>()?;

        Ok(Self::Static(scripts))
    }
}

impl<M> Catalog<'_, M> {
    /// The steps an apply with `options` would run for `status`, rendered
    /// for `dialect`.
    pub fn plan_for<'c>(
        &'c self,
        mut status: Status<'c>,
        dialect: DialectId,
        options: &ApplyOptions,
    ) -> Result<ApplyPlan<'c>, ValidationError> {
        let initial = status.history.is_empty() && status.coverage.is_none();
        let Schedule {
            snapshot,
            entries,
            late,
        } = self.schedule(&status.pending, initial, options, &mut status.blockers);
        let steps = snapshot
            .into_iter()
            .chain(entries)
            .map(|entry| {
                let execution = Execution::of(entry.plan.as_ref(), dialect)?;

                Ok(Step { entry, execution })
            })
            .collect::<Result<_, ValidationError>>()?;
        let covered = snapshot
            .map(|snapshot| snapshot.covers.iter().map(|c| c.version).collect())
            .unwrap_or_default();

        Ok(ApplyPlan {
            status,
            steps,
            covered,
            late,
        })
    }

    /// Selects from the `pending` incrementals what an apply with `options`
    /// runs, removing the late blockers it permits or leaves above its
    /// target version. On an `initial` target, which records no
    /// installation or coverage, it prefers the newest snapshot at or below
    /// the target version whose covered incrementals are all pending, and
    /// runs only the incrementals above it.
    pub(crate) fn schedule<'c>(
        &'c self,
        pending: &[&'c CatalogEntry],
        initial: bool,
        options: &ApplyOptions,
        blockers: &mut Vec<Blocker>,
    ) -> Schedule<'c> {
        let within = |version: Version| options.through.is_none_or(|through| version <= through);
        let covers_pending = |snapshot: &&CatalogEntry| {
            snapshot
                .covers
                .iter()
                .all(|covered| pending.iter().any(|entry| entry.version == covered.version))
        };

        blockers.retain(
            |blocker| !matches!(blocker, Blocker::Late { version, .. } if !within(*version)),
        );

        let late = options.order.permit(blockers);
        let snapshot = match options.init {
            Init::PreferSnapshot if initial => self
                .snapshots()
                .rev()
                .filter(|snapshot| within(snapshot.version))
                .find(covers_pending),
            Init::PreferSnapshot | Init::FullReplay => None,
        };
        let entries = pending
            .iter()
            .copied()
            .filter(|entry| within(entry.version))
            .filter(|entry| snapshot.is_none_or(|snapshot| entry.version > snapshot.version))
            .collect();

        Schedule {
            snapshot,
            entries,
            late,
        }
    }
}
