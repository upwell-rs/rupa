//! Asynchronous migration execution, with the same names as
//! `sync`; the import selects the mode. Futures are `Send` and
//! unboxed.

#[cfg(feature = "sync")]
mod blocking;

use rupa_core::asynchronous::Connection;

use crate::runner::mode;
use crate::{
    ApplyReport, Catalog, CatalogEntry, Error, HistoryEntry, MigrationInfo, Plan, RunError,
    VersionScheme,
};

/// Driver-owned dedicated physical migration connection, awaited.
///
/// The async counterpart of `sync::Session`, with the
/// same obligations. A dropped future cannot await cleanup, so the session
/// must close its dedicated connection when dropped mid-run.
pub trait Session: Connection {
    /// Acquires exclusion, validates/initializes metadata and checks outstanding attempts.
    fn acquire(
        &mut self,
        stream: &str,
        scheme: VersionScheme,
    ) -> impl Future<Output = Result<(), Error<Self::Error>>> + Send;

    /// Reads installed history without executing user code or creating metadata.
    fn history(
        &mut self,
    ) -> impl Future<Output = Result<Vec<HistoryEntry>, Error<Self::Error>>> + Send;

    /// Durably journals an attempt before starting its user transaction.
    fn start_attempt(
        &mut self,
        entry: &CatalogEntry,
    ) -> impl Future<Output = Result<i64, Error<Self::Error>>> + Send;

    /// Opens an artifact transaction after verifying ownership.
    fn begin_migration(&mut self) -> impl Future<Output = Result<(), Error<Self::Error>>> + Send;

    /// Records history/completion and commits them with the artifact's effects.
    fn complete(
        &mut self,
        attempt: i64,
        entry: &CatalogEntry,
    ) -> impl Future<Output = Result<(), Error<Self::Error>>> + Send;

    /// Rolls back an unsuccessful body and records its failure only after acknowledgement.
    fn abort(
        &mut self,
        attempt: i64,
    ) -> impl Future<Output = Result<(), Error<Self::Error>>> + Send;

    /// Releases exclusion only after all transaction and attempt state is known.
    fn release(&mut self) -> impl Future<Output = Result<(), Error<Self::Error>>> + Send;

    /// Executes an authorized user script on this session's current transaction.
    fn script(&mut self, sql: &str) -> impl Future<Output = Result<(), Error<Self::Error>>> + Send;

    /// Applies a catalog while retaining this session for driver-level inspection.
    /// Applications normally use [`Apply::apply`], which owns cleanup.
    fn apply<M: Migration<Self> + Send + Sync>(
        &mut self,
        catalog: &Catalog<'_, M>,
    ) -> impl Future<Output = Result<ApplyReport, RunError<Self::Error, M::Error>>> + Send
    where
        Self: Sized,
    {
        apply(self, catalog)
    }
}

/// Exclusive awaited query/script access without transaction, connection or lock escape hatches.
pub struct Context<'a, S: Session> {
    pub(crate) session: &'a mut S,
    /// The checksummed plan of the running entry, if it is planned.
    pub(crate) plan: Option<&'a Plan>,
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
}

/// What applying a catalog of `M` on session `S` returns.
pub type ApplyResult<S, M> =
    Result<ApplyReport, RunError<<S as Connection>::Error, <M as Migration<S>>::Error>>;

/// Applies a catalog on a dedicated native connection, then closes it.
pub trait Apply {
    /// The catalog's entry type.
    type Entry;

    /// Applies pending migrations; no runner/session wrapper is required, the
    /// driver owns exclusion and cleanup.
    fn apply<S: Session>(
        &self,
        connection: S,
    ) -> impl Future<Output = ApplyResult<S, Self::Entry>> + Send
    where
        Self::Entry: Migration<S>;
}

impl<M: MigrationInfo + Send + Sync> Apply for Catalog<'_, M> {
    type Entry = M;

    async fn apply<S: Session>(
        &self,
        mut connection: S,
    ) -> Result<ApplyReport, RunError<S::Error, M::Error>>
    where
        M: Migration<S>,
    {
        apply(&mut connection, self).await
    }
}

mode!(async);
