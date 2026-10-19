//! The execution engine shared by both modes, compiled only with one of them.

use std::collections::BTreeSet;

use rupa_core::dialect::DialectId;

use crate::{
    ApplyReport, Catalog, Error, HistoryEntry, MigrationInfo, MigrationKind, Policy, RunError,
    ValidationError, Version,
};

impl<E: std::error::Error + 'static, M: std::error::Error + 'static> RunError<E, M> {
    pub(crate) fn infrastructure(source: Error<E>) -> Self {
        Self::Infrastructure {
            source,
            applied: Box::default(),
            cleanup: Vec::new(),
        }
    }

    pub(crate) fn migration(version: Version, source: M) -> Self {
        Self::Migration {
            version,
            source,
            applied: Box::default(),
            cleanup: Vec::new(),
        }
    }

    pub(crate) fn cleanup(&mut self, error: Error<E>) {
        match self {
            Self::Infrastructure { cleanup, .. } | Self::Migration { cleanup, .. } => {
                cleanup.push(error)
            }
        }
    }

    fn with_applied(mut self, versions: Vec<Version>) -> Self {
        match &mut self {
            Self::Infrastructure { applied, .. } | Self::Migration { applied, .. } => {
                *applied = versions.into_boxed_slice()
            }
        }

        self
    }
}

/// Rejects what this runner cannot execute, before the target is touched.
pub(crate) fn preflight<M: MigrationInfo>(
    catalog: &Catalog<'_, M>,
    dialect: DialectId,
) -> Result<(), ValidationError> {
    for entry in &catalog.entries {
        if entry.metadata.kind != MigrationKind::Incremental
            || entry.metadata.policy != Policy::TransactionalRequired
        {
            return Err(ValidationError::Unsupported(entry.metadata.version.into()));
        }
    }

    catalog.validate_plans(dialect)
}

/// The installed versions, after checking history against the catalog:
/// every installed artifact is present and unchanged, and nothing pending
/// sorts below the newest installed version.
pub(crate) fn installed<E: std::error::Error, M>(
    catalog: &Catalog<'_, M>,
    history: &[HistoryEntry],
) -> Result<BTreeSet<Version>, Error<E>> {
    let mut installed = BTreeSet::new();

    for record in history {
        let version = catalog
            .scheme
            .parse(&record.version)
            .map_err(|e| Error::Validation(e.into()))?;
        let entry = catalog
            .entries
            .iter()
            .find(|e| e.version == version)
            .ok_or_else(|| {
                Error::History(format!(
                    "installed migration {version} is missing from the catalog"
                ))
            })?;

        if entry.checksum != record.checksum || !installed.insert(version) {
            return Err(Error::History(format!(
                "installed migration {version} changed checksum or appears twice"
            )));
        }
    }

    if let Some(maximum) = installed.last() {
        for entry in &catalog.entries {
            if entry.version < *maximum && !installed.contains(&entry.version) {
                return Err(Error::History(format!(
                    "late migration {} below installed {maximum}; strict apply refuses it",
                    entry.version
                )));
            }
        }
    }

    Ok(installed)
}

/// Combines a locked run with its release, never letting a release failure
/// replace the run's own error.
pub(crate) fn finish<E: std::error::Error + 'static, M: std::error::Error + 'static>(
    result: Result<(), RunError<E, M>>,
    released: Result<(), Error<E>>,
    applied: Vec<Version>,
) -> Result<ApplyReport, RunError<E, M>> {
    match (result, released) {
        (Ok(()), Ok(())) => Ok(ApplyReport { applied }),
        (Ok(()), Err(source)) => Err(RunError::infrastructure(source).with_applied(applied)),
        (Err(mut error), released) => {
            if let Err(cleanup) = released {
                error.cleanup(cleanup);
            }

            Err(error.with_applied(applied))
        }
    }
}

/// A script prepared for the session: normalized and lexically validated.
pub(crate) fn prepare_script<E: std::error::Error>(
    sql: &str,
    dialect: DialectId,
) -> Result<String, Error<E>> {
    let normalized = crate::catalog::normalize(sql, "script")?;

    crate::validate_script(&normalized, dialect).map_err(|e| Error::Validation(e.into()))?;

    Ok(normalized)
}

/// Emits one mode's execution engine from a single body: the runner, the
/// `Context` methods and the `Planned<T>` adapter. Session calls are suffixed
/// with `.await` in async mode only, so both modes share every step. Expands
/// inside the mode module, whose `Session`, `Migration` and `Context` it uses.
macro_rules! mode {
    (sync) => {
        mode!(@emit [] [] [] []);
    };
    (async) => {
        mode!(@emit [async] [.await] [+ Send + Sync] [+ Send]);
    };
    (@emit [$($async:tt)*] [$($await:tt)*] [$($bound:tt)*] [$($send:tt)*]) => {
        impl<S: Session> Context<'_, S> {
            /// Runs a typed query on the runner-owned transaction.
            pub $($async)* fn run<R: rupa_core::query::QueryResult $($send)*>(
                &mut self,
                query: rupa_core::query::Query<R>,
            ) -> Result<R, crate::Error<S::Error>> {
                self.session.run(query) $($await)* .map_err(crate::Error::Database)
            }

            /// Runs a script using the driver's native parser and migration authorization.
            pub $($async)* fn script(&mut self, sql: &str) -> Result<(), crate::Error<S::Error>> {
                let sql = crate::runner::prepare_script(sql, rupa_core::dialect::Dialect::id(self.session.dialect()))?;

                self.session.script(&sql) $($await)*
            }

            /// Runs a schema operation, rendered for the session's dialect.
            pub $($async)* fn schema(
                &mut self,
                op: &rupa_core::schema::SchemaOp,
            ) -> Result<(), crate::Error<S::Error>> {
                let sql = rupa_sql::render_schema(op, self.session.dialect())
                    .map_err(|e| crate::Error::Validation(e.into()))?;

                self.session.script(&sql) $($await)*
            }
        }

        /// Executes the plan the catalog checksummed when the entry was built,
        /// not a fresh call to `plan()`, so what runs is exactly what was verified.
        impl<S: Session, T: crate::PlannedMigration $($bound)*> Migration<S> for crate::Planned<T> {
            type Error = crate::Error<S::Error>;

            $($async)* fn up(&self, context: &mut Context<'_, S>) -> Result<(), Self::Error> {
                let Some(plan) = context.plan else {
                    return Err(crate::Error::History(
                        "a planned migration ran without its catalog plan".into(),
                    ));
                };

                for operation in plan.operations() {
                    match operation {
                        crate::Operation::Sql(sql) => context.script(sql) $($await)* ?,
                        crate::Operation::Schema(op) => context.schema(op) $($await)* ?,
                    }
                }

                Ok(())
            }
        }

        pub(crate) $($async)* fn apply<S: Session, M: Migration<S> $($bound)*>(
            session: &mut S,
            catalog: &crate::Catalog<'_, M>,
        ) -> Result<crate::ApplyReport, crate::RunError<S::Error, M::Error>> {
            let mut applied = Vec::new();

            crate::runner::preflight(catalog, rupa_core::dialect::Dialect::id(session.dialect()))
                .map_err(|e| crate::RunError::infrastructure(crate::Error::Validation(e)))?;
            session
                .acquire(catalog.stream, catalog.scheme)
                $($await)*
                .map_err(crate::RunError::infrastructure)?;

            let result = apply_locked(session, catalog, &mut applied) $($await)*;
            let released = session.release() $($await)*;

            crate::runner::finish(result, released, applied)
        }

        $($async)* fn apply_locked<S: Session, M: Migration<S> $($bound)*>(
            session: &mut S,
            catalog: &crate::Catalog<'_, M>,
            applied: &mut Vec<crate::Version>,
        ) -> Result<(), crate::RunError<S::Error, M::Error>> {
            let history = session.history() $($await)* .map_err(crate::RunError::infrastructure)?;
            let installed = crate::runner::installed(catalog, &history)
                .map_err(crate::RunError::infrastructure)?;
            let pending = catalog
                .entries
                .iter()
                .filter(|entry| !installed.contains(&entry.version));

            for entry in pending {
                let attempt = session
                    .start_attempt(entry)
                    $($await)*
                    .map_err(crate::RunError::infrastructure)?;

                if let Err(source) = session.begin_migration() $($await)* {
                    return Err(
                        abort(session, attempt, crate::RunError::infrastructure(source)) $($await)*
                    );
                }

                let mut context = Context {
                    session: &mut *session,
                    plan: entry.plan.as_ref(),
                };

                if let Err(source) = catalog.migrations[entry.index].up(&mut context) $($await)* {
                    return Err(
                        abort(session, attempt, crate::RunError::migration(entry.version, source))
                            $($await)*
                    );
                }

                if let Err(source) = session.complete(attempt, entry) $($await)* {
                    return Err(
                        abort(session, attempt, crate::RunError::infrastructure(source)) $($await)*
                    );
                }

                applied.push(entry.version);
            }

            Ok(())
        }

        /// Rolls back an unsuccessful attempt, keeping cleanup failures beside the original error.
        $($async)* fn abort<S: Session, M: std::error::Error + Send + 'static>(
            session: &mut S,
            attempt: i64,
            mut error: crate::RunError<S::Error, M>,
        ) -> crate::RunError<S::Error, M> {
            if let Err(cleanup) = session.abort(attempt) $($await)* {
                error.cleanup(cleanup);
            }

            error
        }
    };
}

pub(crate) use mode;
