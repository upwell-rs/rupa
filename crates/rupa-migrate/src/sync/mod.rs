//! Synchronous migration execution. The same names exist in
//! `asynchronous`; the import selects the mode.

use rupa_core::dialect::Dialect;
use rupa_core::sync::Connection;

use crate::runner::mode;
use crate::{
    AdoptReport, Adoption, ApplyOptions, ApplyPlan, ApplyReport, Catalog, CatalogEntry, Coverage,
    Covered, Direction, Error, HistoryEntry, Inspection, MigrationInfo, Plan, Policy, Recovery,
    RevertPlan, RevertReport, RevertTarget, RunError, Status, VersionScheme,
};

/// Driver-owned dedicated physical migration connection.
///
/// User code receives only [`Context`], never this control surface. Implementors
/// must atomically record transactional effects/history and retain exclusion
/// across artifact commits. Dropping an interrupted session must close its
/// connection; an unresolved durable claim must never expire automatically.
pub trait Session: Connection {
    /// The execution policies this session can honour. A catalog containing
    /// any other policy is rejected before the target is touched; a policy is
    /// never downgraded.
    const POLICIES: &'static [Policy] = &[Policy::TransactionalRequired];

    /// Acquires exclusion, validates/initializes metadata and checks
    /// outstanding attempts. Returns the interrupted transactional attempts it
    /// recorded as rolled back, after which `apply` stops with
    /// [`Error::Reconciled`]; an unresolved nontransactional attempt fails
    /// with [`Error::Uncertain`].
    fn acquire(
        &mut self,
        stream: &str,
        scheme: VersionScheme,
    ) -> Result<Vec<i64>, Error<Self::Error>>;

    /// Reads installed history without executing user code or creating metadata.
    fn history(&mut self) -> Result<Vec<HistoryEntry>, Error<Self::Error>>;

    /// Reads the target's coverage claim, if any, without executing user
    /// code or creating metadata.
    fn coverage(&mut self) -> Result<Option<Coverage>, Error<Self::Error>>;

    /// Reads the target's journal and whether another runner excludes it,
    /// without acquiring exclusion, writing or executing user code. Only
    /// possible on a session that owns no run.
    fn inspect(&mut self) -> Result<Inspection, Error<Self::Error>>;

    /// Durably journals an attempt of `entry` in `direction` before starting
    /// its user transaction.
    fn start_attempt(
        &mut self,
        entry: &CatalogEntry,
        direction: Direction,
    ) -> Result<i64, Error<Self::Error>>;

    /// Prepares the body of a journaled attempt under `policy` after verifying
    /// ownership: an artifact transaction, or autocommit for nontransactional work.
    fn begin_migration(&mut self, policy: Policy) -> Result<(), Error<Self::Error>>;

    /// Records history/completion and commits them with the artifact's
    /// effects. A completed down attempt makes the installation it undoes
    /// inactive; a completed snapshot records its coverage manifest.
    fn complete(&mut self, attempt: i64, entry: &CatalogEntry) -> Result<(), Error<Self::Error>>;

    /// Rolls back an unsuccessful body and records its failure only after
    /// acknowledgement. A nontransactional body cannot be rolled back: its
    /// attempt stays unresolved and this reports [`Error::Uncertain`].
    fn abort(&mut self, attempt: i64) -> Result<(), Error<Self::Error>>;

    /// Releases exclusion only after all transaction and attempt state is known.
    fn release(&mut self) -> Result<(), Error<Self::Error>>;

    /// Executes an authorized user script on this session's current transaction.
    fn script(&mut self, sql: &str) -> Result<(), Error<Self::Error>>;

    /// Records an operator's `recovery` assertion under newly acquired
    /// exclusion, with `entries` as the catalog the assertion is checked
    /// against, then releases exclusion. Executes no body.
    fn recover(
        &mut self,
        stream: &str,
        scheme: VersionScheme,
        entries: &[CatalogEntry],
        recovery: &Recovery<'_>,
    ) -> Result<(), Error<Self::Error>>;

    /// Records an operator's `adoption` of an unmanaged target under newly
    /// acquired exclusion, then releases exclusion: creates the journal with
    /// the adoption event and its `members` in one transaction. Refuses a
    /// fresh or managed target. Executes no body.
    fn adopt(
        &mut self,
        stream: &str,
        scheme: VersionScheme,
        adoption: &Adoption<'_>,
        members: &[Covered],
    ) -> Result<(), Error<Self::Error>>;

    /// Marks the session unusable after a run ended without returning (a
    /// dropped future or an unwinding panic). Called from `Drop`, so it must
    /// not block or perform IO; dropping the session must then close its
    /// connection, which ends any transaction or session lock.
    fn abandon(&mut self);

    /// Applies a catalog while retaining this session for driver-level inspection.
    /// Applications normally use [`Apply::apply`], which owns cleanup.
    fn apply<M: Migration<Self>>(
        &mut self,
        catalog: &Catalog<'_, M>,
    ) -> Result<ApplyReport, RunError<Self::Error, M::Error>>
    where
        Self: Sized,
    {
        apply(self, catalog, &ApplyOptions::DEFAULT)
    }

    /// [`Session::apply`] with per-run `options`.
    fn apply_with<M: Migration<Self>>(
        &mut self,
        catalog: &Catalog<'_, M>,
        options: &ApplyOptions,
    ) -> Result<ApplyReport, RunError<Self::Error, M::Error>>
    where
        Self: Sized,
    {
        apply(self, catalog, options)
    }

    /// Reverts installations while retaining this session for driver-level
    /// inspection. Applications normally use [`Revert::revert`].
    fn revert<M: Migration<Self>>(
        &mut self,
        catalog: &Catalog<'_, M>,
        target: RevertTarget,
    ) -> RevertResult<Self, M>
    where
        Self: Sized,
    {
        revert(self, catalog, target)
    }
}

/// Exclusive query/script access without transaction, connection or lock escape hatches.
pub struct Context<'a, S: Session> {
    pub(crate) session: &'a mut S,
    /// The checksummed plan of the running direction, if the entry is planned.
    pub(crate) plan: Option<&'a Plan>,
    /// Set by the default [`Migration::down`]: the entry declared a down
    /// body its impl does not provide.
    pub(crate) missing_down: bool,
}

/// A compiled sync migration body. Its error remains statically typed.
pub trait Migration<S: Session>: MigrationInfo {
    /// Application error, preserving its driver/session cause.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Runs forward changes using the selected session, without automatic retries.
    fn up(&self, context: &mut Context<'_, S>) -> Result<(), Self::Error>;

    /// Undoes [`up`](Self::up), without automatic retries. A revert calls it
    /// only for an entry whose [`MigrationInfo::reversal`] is
    /// [`Reversal::Imperative`](crate::Reversal::Imperative), which rupa-build
    /// declares when the impl defines `down`. Without an override, such a
    /// revert fails with [`Error::MissingDown`] and rolls back.
    fn down(&self, context: &mut Context<'_, S>) -> Result<(), Self::Error> {
        context.missing_down = true;

        Ok(())
    }
}

/// What applying a catalog of `M` on session `S` returns.
pub type ApplyResult<S, M> =
    Result<ApplyReport, RunError<<S as Connection>::Error, <M as Migration<S>>::Error>>;

/// What reverting a catalog of `M` on session `S` returns.
pub type RevertResult<S, M> = Result<
    RevertReport,
    RunError<<S as Connection>::Error, <M as Migration<S>>::Error, RevertReport>,
>;

/// Applies a catalog on a dedicated native connection, then closes it.
pub trait Apply {
    /// The catalog's entry type.
    type Entry;

    /// Applies pending migrations under `options`; no runner/session
    /// wrapper is required, the driver owns exclusion and cleanup.
    fn apply_with<S: Session>(
        &self,
        connection: S,
        options: &ApplyOptions,
    ) -> ApplyResult<S, Self::Entry>
    where
        Self::Entry: Migration<S>;

    /// Applies pending migrations strictly, refusing late artifacts.
    fn apply<S: Session>(&self, connection: S) -> ApplyResult<S, Self::Entry>
    where
        Self::Entry: Migration<S>,
    {
        self.apply_with(connection, &ApplyOptions::DEFAULT)
    }
}

/// Undoes installations on a dedicated native connection, then closes it.
pub trait Revert {
    /// The catalog's entry type.
    type Entry;

    /// Undoes the newest installations `target` selects, newest first, each
    /// committed on its own. Every selected installation must be reversible
    /// and history must match the catalog, or nothing runs.
    fn revert<S: Session>(
        &self,
        connection: S,
        target: RevertTarget,
    ) -> RevertResult<S, Self::Entry>
    where
        Self::Entry: Migration<S>;
}

/// Records an operator's recovery assertion on a dedicated native
/// connection, then closes it.
pub trait Recover {
    /// Records `recovery` under exclusion, checked against this catalog.
    fn recover<S: Session>(
        &self,
        connection: S,
        recovery: &Recovery<'_>,
    ) -> Result<(), Error<S::Error>>;
}

/// Records an operator's adoption on a dedicated native connection, then
/// closes it.
pub trait Adopt {
    /// Records `adoption` of this catalog's incrementals at or below its
    /// boundary under exclusion, without running any body.
    fn adopt<S: Session>(
        &self,
        connection: S,
        adoption: &Adoption<'_>,
    ) -> Result<AdoptReport, Error<S::Error>>;
}

/// Reads a target on a dedicated native connection, then closes it, without
/// acquiring exclusion, writing or executing a body.
pub trait Inspect {
    /// This catalog against the target.
    fn status<S: Session>(&self, connection: S) -> Result<Status<'_>, Error<S::Error>>;

    /// What an apply with `options` would run on the target: its status and
    /// the pending steps, rendered for the session's dialect after the checks
    /// apply runs first.
    fn plan_with<S: Session>(
        &self,
        connection: S,
        options: &ApplyOptions,
    ) -> Result<ApplyPlan<'_>, Error<S::Error>>;

    /// What a revert to `target` would undo on the target: its status and
    /// the steps, rendered for the session's dialect after the checks a
    /// revert runs first.
    fn revert_plan<S: Session>(
        &self,
        connection: S,
        target: RevertTarget,
    ) -> Result<RevertPlan<'_>, Error<S::Error>>;

    /// What a strict apply would run on the target.
    fn plan<S: Session>(&self, connection: S) -> Result<ApplyPlan<'_>, Error<S::Error>> {
        self.plan_with(connection, &ApplyOptions::DEFAULT)
    }
}

impl<M: MigrationInfo> Inspect for Catalog<'_, M> {
    fn status<S: Session>(&self, mut connection: S) -> Result<Status<'_>, Error<S::Error>> {
        Ok(self.classify(inspect(&mut connection)?))
    }

    fn plan_with<S: Session>(
        &self,
        connection: S,
        options: &ApplyOptions,
    ) -> Result<ApplyPlan<'_>, Error<S::Error>> {
        let dialect = Dialect::id(connection.dialect());

        self.check(dialect, S::POLICIES)?;

        let status = self.status(connection)?;

        Ok(self.plan_for(status, dialect, options)?)
    }

    fn revert_plan<S: Session>(
        &self,
        connection: S,
        target: RevertTarget,
    ) -> Result<RevertPlan<'_>, Error<S::Error>> {
        let dialect = Dialect::id(connection.dialect());

        self.check(dialect, S::POLICIES)?;

        let status = self.status(connection)?;

        Ok(self.revert_for(status, dialect, target)?)
    }
}

impl<M> Recover for Catalog<'_, M> {
    fn recover<S: Session>(
        &self,
        mut connection: S,
        recovery: &Recovery<'_>,
    ) -> Result<(), Error<S::Error>> {
        recover(
            &mut connection,
            self.stream,
            self.scheme,
            &self.entries,
            recovery,
        )
    }
}

impl<M> Adopt for Catalog<'_, M> {
    fn adopt<S: Session>(
        &self,
        mut connection: S,
        adoption: &Adoption<'_>,
    ) -> Result<AdoptReport, Error<S::Error>> {
        adopt(&mut connection, self, adoption)
    }
}

impl<M: MigrationInfo> Apply for Catalog<'_, M> {
    type Entry = M;

    fn apply_with<S: Session>(
        &self,
        mut connection: S,
        options: &ApplyOptions,
    ) -> Result<ApplyReport, RunError<S::Error, M::Error>>
    where
        M: Migration<S>,
    {
        apply(&mut connection, self, options)
    }
}

impl<M: MigrationInfo> Revert for Catalog<'_, M> {
    type Entry = M;

    fn revert<S: Session>(&self, mut connection: S, target: RevertTarget) -> RevertResult<S, M>
    where
        M: Migration<S>,
    {
        revert(&mut connection, self, target)
    }
}

mode!(sync);
