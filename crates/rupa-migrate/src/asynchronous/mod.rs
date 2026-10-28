//! Asynchronous migration execution, with the same names as
//! `sync`; the import selects the mode. Futures are `Send` and
//! unboxed.

#[cfg(feature = "sync")]
mod blocking;

use rupa_core::dialect::Dialect;
use rupa_core::exec::{AsyncExecutor, Outcome};
use rupa_core::ir::Statement;
use rupa_core::query::Expect;

use crate::runner::mode;
use crate::{
    AdoptReport, Adoption, ApplyOptions, ApplyPlan, ApplyReport, Catalog, CatalogEntry, Coverage,
    Covered, Direction, Error, HistoryEntry, Inspection, MigrationInfo, Plan, Policy, Recovery,
    RevertPlan, RevertReport, RevertTarget, RunError, Status, VersionScheme,
};

/// Driver-owned dedicated physical migration connection, awaited.
///
/// The async counterpart of `sync::Session`, with the
/// same obligations. A dropped future cannot await cleanup, so the session
/// must close its dedicated connection when dropped mid-run.
pub trait Session: AsyncExecutor {
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
    ) -> impl Future<Output = Result<Vec<i64>, Error<Self::Error>>> + Send;

    /// Reads installed history without executing user code or creating metadata.
    fn history(
        &mut self,
    ) -> impl Future<Output = Result<Vec<HistoryEntry>, Error<Self::Error>>> + Send;

    /// Reads the target's coverage claim, if any, without executing user
    /// code or creating metadata.
    fn coverage(
        &mut self,
    ) -> impl Future<Output = Result<Option<Coverage>, Error<Self::Error>>> + Send;

    /// Whether the journal records a completed installation or coverage
    /// claim, even one since undone, without executing user code or creating
    /// metadata. Until it does, apply may initialize the target from a snapshot.
    fn initialized(&mut self) -> impl Future<Output = Result<bool, Error<Self::Error>>> + Send;

    /// Reads the target's journal and whether another runner excludes it,
    /// without acquiring exclusion, writing or executing user code. Only
    /// possible on a session that owns no run.
    fn inspect(&mut self) -> impl Future<Output = Result<Inspection, Error<Self::Error>>> + Send;

    /// Durably journals an attempt of `entry` in `direction` before starting
    /// its user transaction.
    fn start_attempt(
        &mut self,
        entry: &CatalogEntry,
        direction: Direction,
    ) -> impl Future<Output = Result<i64, Error<Self::Error>>> + Send;

    /// Prepares the body of a journaled attempt under `policy` after verifying
    /// ownership: an artifact transaction, or autocommit for nontransactional work.
    fn begin_migration(
        &mut self,
        policy: Policy,
    ) -> impl Future<Output = Result<(), Error<Self::Error>>> + Send;

    /// Records history/completion and commits them with the artifact's
    /// effects. A completed down attempt makes the installation it undoes
    /// inactive; a completed snapshot records its coverage manifest.
    fn complete(
        &mut self,
        attempt: i64,
        entry: &CatalogEntry,
    ) -> impl Future<Output = Result<(), Error<Self::Error>>> + Send;

    /// Rolls back an unsuccessful body and records its failure only after
    /// acknowledgement. A nontransactional body cannot be rolled back: its
    /// attempt stays unresolved and this reports [`Error::Uncertain`].
    fn abort(
        &mut self,
        attempt: i64,
    ) -> impl Future<Output = Result<(), Error<Self::Error>>> + Send;

    /// Releases exclusion only after all transaction and attempt state is known.
    fn release(&mut self) -> impl Future<Output = Result<(), Error<Self::Error>>> + Send;

    /// Executes an authorized user script on this session's current transaction.
    fn script(&mut self, sql: &str) -> impl Future<Output = Result<(), Error<Self::Error>>> + Send;

    /// Runs a typed statement from a migration body, under the same
    /// authorization as its scripts, inside the attempt's transaction.
    fn user_statement(
        &mut self,
        statement: &Statement,
        expect: Expect,
    ) -> impl Future<Output = Result<Outcome, Self::Error>> + Send;

    /// Records an operator's `recovery` assertion under newly acquired
    /// exclusion, with `entries` as the catalog the assertion is checked
    /// against, then releases exclusion. Executes no body.
    fn recover(
        &mut self,
        stream: &str,
        scheme: VersionScheme,
        entries: &[CatalogEntry],
        recovery: &Recovery<'_>,
    ) -> impl Future<Output = Result<(), Error<Self::Error>>> + Send;

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
    ) -> impl Future<Output = Result<(), Error<Self::Error>>> + Send;

    /// Marks the session unusable after a run ended without returning (a
    /// dropped future or an unwinding panic). Called from `Drop`, so it must
    /// not block or perform IO; dropping the session must then close its
    /// connection, which ends any transaction or session lock.
    fn abandon(&mut self);

    /// Applies a catalog while retaining this session for driver-level inspection.
    /// Applications normally use [`Apply::apply`], which owns cleanup.
    fn apply<M: Migration<Self> + Send + Sync>(
        &mut self,
        catalog: &Catalog<'_, M>,
    ) -> impl Future<Output = Result<ApplyReport, RunError<Self::Error, M::Error>>> + Send
    where
        Self: Sized,
    {
        apply(self, catalog, &ApplyOptions::DEFAULT)
    }

    /// [`Session::apply`] with per-run `options`.
    fn apply_with<M: Migration<Self> + Send + Sync>(
        &mut self,
        catalog: &Catalog<'_, M>,
        options: &ApplyOptions,
    ) -> impl Future<Output = Result<ApplyReport, RunError<Self::Error, M::Error>>> + Send
    where
        Self: Sized,
    {
        apply(self, catalog, options)
    }

    /// Reverts installations while retaining this session for driver-level
    /// inspection. Applications normally use [`Revert::revert`].
    fn revert<M: Migration<Self> + Send + Sync>(
        &mut self,
        catalog: &Catalog<'_, M>,
        target: RevertTarget,
    ) -> impl Future<Output = RevertResult<Self, M>> + Send
    where
        Self: Sized,
    {
        revert(self, catalog, target)
    }
}

/// Exclusive awaited query/script access without transaction, connection or lock escape hatches.
pub struct Context<'a, S: Session> {
    pub(crate) session: futures_util::lock::Mutex<&'a mut S>,
    /// The checksummed plan of the running direction, if the entry is planned.
    pub(crate) plan: Option<&'a Plan>,
    /// Set by the default [`Migration::down`]: the entry declared a down
    /// body its impl does not provide.
    pub(crate) missing_down: bool,
}

/// A compiled async migration body. Its error remains statically typed.
pub trait Migration<S: Session>: MigrationInfo {
    /// Application error, preserving its driver/session cause.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Runs forward changes using the selected session, without automatic retries.
    fn up(
        &self,
        context: &mut Context<'_, S>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send;

    /// Undoes [`up`](Self::up), without automatic retries. A revert calls it
    /// only for an entry whose [`MigrationInfo::reversal`] is
    /// [`Reversal::Imperative`](crate::Reversal::Imperative), which rupa-build
    /// declares when the impl defines `down`. Without an override, such a
    /// revert fails with [`Error::MissingDown`] and rolls back.
    fn down(
        &self,
        context: &mut Context<'_, S>,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send {
        context.missing_down = true;

        async { Ok(()) }
    }
}

/// What applying a catalog of `M` on session `S` returns.
pub type ApplyResult<S, M> =
    Result<ApplyReport, RunError<<S as AsyncExecutor>::Error, <M as Migration<S>>::Error>>;

/// What reverting a catalog of `M` on session `S` returns.
pub type RevertResult<S, M> = Result<
    RevertReport,
    RunError<<S as AsyncExecutor>::Error, <M as Migration<S>>::Error, RevertReport>,
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
    ) -> impl Future<Output = ApplyResult<S, Self::Entry>> + Send
    where
        Self::Entry: Migration<S>;

    /// Applies pending migrations strictly, refusing late artifacts.
    fn apply<S: Session>(
        &self,
        connection: S,
    ) -> impl Future<Output = ApplyResult<S, Self::Entry>> + Send
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
    ) -> impl Future<Output = RevertResult<S, Self::Entry>> + Send
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
    ) -> impl Future<Output = Result<(), Error<S::Error>>> + Send;
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
    ) -> impl Future<Output = Result<AdoptReport, Error<S::Error>>> + Send;
}

/// Reads a target on a dedicated native connection, then closes it, without
/// acquiring exclusion, writing or executing a body.
pub trait Inspect {
    /// This catalog against the target.
    fn status<S: Session>(
        &self,
        connection: S,
    ) -> impl Future<Output = Result<Status<'_>, Error<S::Error>>> + Send;

    /// What an apply with `options` would run on the target: its status and
    /// the pending steps, rendered for the session's dialect after the checks
    /// apply runs first.
    fn plan_with<S: Session>(
        &self,
        connection: S,
        options: &ApplyOptions,
    ) -> impl Future<Output = Result<ApplyPlan<'_>, Error<S::Error>>> + Send;

    /// What a revert to `target` would undo on the target: its status and
    /// the steps, rendered for the session's dialect after the checks a
    /// revert runs first.
    fn revert_plan<S: Session>(
        &self,
        connection: S,
        target: RevertTarget,
    ) -> impl Future<Output = Result<RevertPlan<'_>, Error<S::Error>>> + Send;

    /// What a strict apply would run on the target.
    fn plan<S: Session>(
        &self,
        connection: S,
    ) -> impl Future<Output = Result<ApplyPlan<'_>, Error<S::Error>>> + Send {
        self.plan_with(connection, &ApplyOptions::DEFAULT)
    }
}

impl<M: MigrationInfo + Sync> Inspect for Catalog<'_, M> {
    async fn status<S: Session>(&self, mut connection: S) -> Result<Status<'_>, Error<S::Error>> {
        Ok(self.classify(inspect(&mut connection).await?))
    }

    async fn plan_with<S: Session>(
        &self,
        connection: S,
        options: &ApplyOptions,
    ) -> Result<ApplyPlan<'_>, Error<S::Error>> {
        let dialect = Dialect::id(connection.dialect());

        self.check(dialect, S::POLICIES)?;

        let status = self.status(connection).await?;

        Ok(self.plan_for(status, dialect, options)?)
    }

    async fn revert_plan<S: Session>(
        &self,
        connection: S,
        target: RevertTarget,
    ) -> Result<RevertPlan<'_>, Error<S::Error>> {
        let dialect = Dialect::id(connection.dialect());

        self.check(dialect, S::POLICIES)?;

        let status = self.status(connection).await?;

        Ok(self.revert_for(status, dialect, target)?)
    }
}

impl<M> Recover for Catalog<'_, M> {
    fn recover<S: Session>(
        &self,
        mut connection: S,
        recovery: &Recovery<'_>,
    ) -> impl Future<Output = Result<(), Error<S::Error>>> + Send {
        let (stream, scheme, entries) = (self.stream, self.scheme, &self.entries);

        async move { recover(&mut connection, stream, scheme, entries, recovery).await }
    }
}

impl<M: Sync> Adopt for Catalog<'_, M> {
    async fn adopt<S: Session>(
        &self,
        mut connection: S,
        adoption: &Adoption<'_>,
    ) -> Result<AdoptReport, Error<S::Error>> {
        adopt(&mut connection, self, adoption).await
    }
}

impl<M: MigrationInfo + Send + Sync> Apply for Catalog<'_, M> {
    type Entry = M;

    async fn apply_with<S: Session>(
        &self,
        mut connection: S,
        options: &ApplyOptions,
    ) -> Result<ApplyReport, RunError<S::Error, M::Error>>
    where
        M: Migration<S>,
    {
        apply(&mut connection, self, options).await
    }
}

impl<M: MigrationInfo + Send + Sync> Revert for Catalog<'_, M> {
    type Entry = M;

    async fn revert<S: Session>(
        &self,
        mut connection: S,
        target: RevertTarget,
    ) -> RevertResult<S, M>
    where
        M: Migration<S>,
    {
        revert(&mut connection, self, target).await
    }
}

mode!(async);
