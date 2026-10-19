use rupa_core::dialect::Dialect;
use rupa_core::exec::Executor;
use rupa_core::query::{Query, QueryResult};
use rupa_core::schema::SchemaOp;

use crate::{
    Catalog, CatalogEntry, Checksum, MigrationInfo, Operation, Plan, Planned, PlannedMigration,
    ValidationError, VersionScheme,
};

/// Migration infrastructure failed without erasing the driver's error type.
#[derive(Debug, thiserror::Error)]
pub enum Error<E: std::error::Error + 'static> {
    /// A driver operation failed.
    #[error("migration database operation failed: {0}")]
    Database(#[source] E),
    /// An artifact cannot be safely interpreted.
    #[error(transparent)]
    Validation(#[from] ValidationError),
    /// Ownership, schema or history no longer matches the runner's assumptions.
    #[error("invalid migration history: {0}")]
    History(String),
    /// A cooperative runner already owns the target, possibly after interruption.
    #[error(
        "migration target has an existing owner; stop the old runner and reconcile before recovery"
    )]
    Busy,
    /// A nonempty database must not be initialized implicitly.
    #[error("unmanaged target contains user objects; explicit adoption is required")]
    Unmanaged,
    /// An interrupted attempt prevents blind replay.
    #[error("migration attempt {0} is unresolved; explicit reconciliation is required")]
    Uncertain(i64),
}

/// Durable successfully executed incremental history.
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    /// Strictly increasing installation order, separate from version order.
    pub installation_rank: u64,
    /// Canonical logical version.
    pub version: String,
    /// Checksum of the installed source closure.
    pub checksum: Checksum,
}

/// Driver-owned dedicated physical migration connection.
///
/// User code receives only [`Context`], never this control surface. Implementors
/// must atomically record transactional effects/history and retain exclusion
/// across artifact commits. Dropping an interrupted session must close its
/// connection; an unresolved durable claim must never expire automatically.
pub trait Session: Executor {
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
    /// Applications normally use `catalog.apply(connection)` to own cleanup.
    fn apply<M: Migration<Self>>(
        &mut self,
        catalog: &Catalog<'_, M>,
    ) -> Result<crate::ApplyReport, crate::RunError<Self::Error, M::Error>>
    where
        Self: Sized,
    {
        crate::runner::apply(self, catalog)
    }
}

/// Exclusive query/script access without transaction, connection or lock escape hatches.
pub struct Context<'a, S: Session> {
    pub(crate) session: &'a mut S,
    /// The checksummed plan of the running entry, if it is planned.
    pub(crate) plan: Option<&'a Plan>,
}

impl<S: Session> Context<'_, S> {
    /// Runs a typed query on the runner-owned transaction.
    pub fn run<R: QueryResult>(&mut self, query: Query<R>) -> Result<R, Error<S::Error>> {
        self.session.run(query).map_err(Error::Database)
    }

    /// Runs a script using the driver's native parser and migration authorization.
    pub fn script(&mut self, sql: &str) -> Result<(), Error<S::Error>> {
        let normalized = crate::catalog::normalize(sql, "script")?;
        crate::script::validate(&normalized, self.session.dialect().id())?;

        self.session.script(&normalized)
    }

    /// Runs a schema operation, rendered for the session's dialect.
    pub fn schema(&mut self, op: &SchemaOp) -> Result<(), Error<S::Error>> {
        let sql = rupa_sql::render_schema(op, self.session.dialect())
            .map_err(|e| Error::Validation(e.into()))?;

        self.session.script(&sql)
    }
}

/// A compiled sync migration body. Its error remains statically typed.
pub trait Migration<S: Session>: MigrationInfo {
    /// Application error, preserving its driver/session cause.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Runs forward changes using the selected session, without automatic retries.
    fn up(&self, context: &mut Context<'_, S>) -> Result<(), Self::Error>;
}

/// Executes the plan the catalog checksummed when the entry was built, not a
/// fresh call to `plan()`, so what runs is exactly what was verified.
impl<S: Session, T: PlannedMigration> Migration<S> for Planned<T> {
    type Error = Error<S::Error>;

    fn up(&self, context: &mut Context<'_, S>) -> Result<(), Self::Error> {
        let Some(plan) = context.plan else {
            return Err(Error::History(
                "a planned migration ran without its catalog plan".into(),
            ));
        };

        for operation in plan.operations() {
            match operation {
                Operation::Sql(sql) => context.script(sql)?,
                Operation::Schema(op) => context.schema(op)?,
            }
        }

        Ok(())
    }
}
