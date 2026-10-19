//! Synchronous migration execution. The same names exist in
//! [`asynchronous`](crate::asynchronous); the import selects the mode.

use rupa_core::sync::Connection;

use crate::runner::mode;
use crate::{
    ApplyReport, Catalog, CatalogEntry, Error, HistoryEntry, MigrationInfo, Plan, RunError,
    VersionScheme,
};

/// Driver-owned dedicated physical migration connection.
///
/// User code receives only [`Context`], never this control surface. Implementors
/// must atomically record transactional effects/history and retain exclusion
/// across artifact commits. Dropping an interrupted session must close its
/// connection; an unresolved durable claim must never expire automatically.
pub trait Session: Connection {
    /// Acquires exclusion, validates/initializes metadata and checks outstanding attempts.
    fn acquire(&mut self, stream: &str, scheme: VersionScheme) -> Result<(), Error<Self::Error>>;

    /// Reads installed history without executing user code or creating metadata.
    fn history(&mut self) -> Result<Vec<HistoryEntry>, Error<Self::Error>>;

    /// Durably journals an attempt before starting its user transaction.
    fn start_attempt(&mut self, entry: &CatalogEntry) -> Result<i64, Error<Self::Error>>;

    /// Opens an artifact transaction after verifying ownership.
    fn begin_migration(&mut self) -> Result<(), Error<Self::Error>>;

    /// Records history/completion and commits them with the artifact's effects.
    fn complete(&mut self, attempt: i64, entry: &CatalogEntry) -> Result<(), Error<Self::Error>>;

    /// Rolls back an unsuccessful body and records its failure only after acknowledgement.
    fn abort(&mut self, attempt: i64) -> Result<(), Error<Self::Error>>;

    /// Releases exclusion only after all transaction and attempt state is known.
    fn release(&mut self) -> Result<(), Error<Self::Error>>;

    /// Executes an authorized user script on this session's current transaction.
    fn script(&mut self, sql: &str) -> Result<(), Error<Self::Error>>;

    /// Applies a catalog while retaining this session for driver-level inspection.
    /// Applications normally use [`Apply::apply`], which owns cleanup.
    fn apply<M: Migration<Self>>(
        &mut self,
        catalog: &Catalog<'_, M>,
    ) -> Result<ApplyReport, RunError<Self::Error, M::Error>>
    where
        Self: Sized,
    {
        apply(self, catalog)
    }
}

/// Exclusive query/script access without transaction, connection or lock escape hatches.
pub struct Context<'a, S: Session> {
    pub(crate) session: &'a mut S,
    /// The checksummed plan of the running entry, if it is planned.
    pub(crate) plan: Option<&'a Plan>,
}

/// A compiled sync migration body. Its error remains statically typed.
pub trait Migration<S: Session>: MigrationInfo {
    /// Application error, preserving its driver/session cause.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Runs forward changes using the selected session, without automatic retries.
    fn up(&self, context: &mut Context<'_, S>) -> Result<(), Self::Error>;
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
    fn apply<S: Session>(&self, connection: S) -> ApplyResult<S, Self::Entry>
    where
        Self::Entry: Migration<S>;
}

impl<M: MigrationInfo> Apply for Catalog<'_, M> {
    type Entry = M;

    fn apply<S: Session>(
        &self,
        mut connection: S,
    ) -> Result<ApplyReport, RunError<S::Error, M::Error>>
    where
        M: Migration<S>,
    {
        apply(&mut connection, self)
    }
}

mode!(sync);
