//! The execution engine shared by both modes, compiled only with one of them.

use rupa_core::dialect::DialectId;

use crate::{ApplyReport, Catalog, CatalogEntry, Error, HistoryEntry, Order, RunError, Version};

impl<E: std::error::Error + 'static, M: std::error::Error + 'static> RunError<E, M> {
    pub(crate) fn infrastructure(source: Error<E>) -> Self {
        Self::Infrastructure {
            source,
            report: Box::default(),
            cleanup: Vec::new(),
        }
    }

    pub(crate) fn migration(version: Version, source: M) -> Self {
        Self::Migration {
            version,
            source,
            report: Box::default(),
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

    fn with_report(mut self, committed: ApplyReport) -> Self {
        match &mut self {
            Self::Infrastructure { report, .. } | Self::Migration { report, .. } => {
                **report = committed
            }
        }

        self
    }
}

/// The catalog entries an apply runs on a target with `history`, in
/// version order.
pub(crate) struct Pending<'c> {
    /// The entries history does not record.
    pub(crate) entries: Vec<&'c CatalogEntry>,
    /// Those below an installed version, which `order` permits.
    pub(crate) late: Vec<Version>,
}

/// The pending entries, after checking history against the catalog with the
/// comparison status uses: every installed artifact is present, unchanged and
/// recorded once, and nothing pending is late unless `order` permits it.
pub(crate) fn pending<'c, E: std::error::Error, M>(
    catalog: &'c Catalog<'_, M>,
    history: &[HistoryEntry],
    order: Order,
) -> Result<Pending<'c>, Error<E>> {
    let mut blockers = Vec::new();
    let installed = catalog.compare(history, &mut blockers);
    let late = order.permit(&mut blockers);

    if !blockers.is_empty() {
        return Err(Error::Blocked(blockers));
    }

    let entries = catalog
        .entries
        .iter()
        .filter(|entry| !installed.contains(&entry.version))
        .collect();

    Ok(Pending { entries, late })
}

/// Combines a locked run with its release, never letting a release failure
/// replace the run's own error.
pub(crate) fn finish<E: std::error::Error + 'static, M: std::error::Error + 'static>(
    result: Result<(), RunError<E, M>>,
    released: Result<(), Error<E>>,
    report: ApplyReport,
) -> Result<ApplyReport, RunError<E, M>> {
    match (result, released) {
        (Ok(()), Ok(())) => Ok(report),
        (Ok(()), Err(source)) => Err(RunError::infrastructure(source).with_report(report)),
        (Err(mut error), released) => {
            if let Err(cleanup) = released {
                error.cleanup(cleanup);
            }

            Err(error.with_report(report))
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
            options: &crate::ApplyOptions,
        ) -> Result<crate::ApplyReport, crate::RunError<S::Error, M::Error>> {
            let mut report = crate::ApplyReport::default();

            catalog
                .check(rupa_core::dialect::Dialect::id(session.dialect()), S::POLICIES)
                .map_err(|e| crate::RunError::infrastructure(crate::Error::Validation(e)))?;

            let mut guard = RunGuard { session, armed: true };
            let reconciled = match guard.session.acquire(catalog.stream, catalog.scheme) $($await)* {
                Ok(reconciled) => reconciled,
                Err(source) => {
                    guard.armed = false;

                    return Err(crate::RunError::infrastructure(source));
                }
            };

            let result = if reconciled.is_empty() {
                apply_locked(&mut *guard.session, catalog, options, &mut report) $($await)*
            } else {
                Err(crate::RunError::infrastructure(crate::Error::Reconciled(reconciled)))
            };
            let released = guard.session.release() $($await)*;

            guard.armed = false;

            crate::runner::finish(result, released, report)
        }

        /// Records `recovery` against `entries`, abandoning the session if the
        /// call ends without returning.
        pub(crate) $($async)* fn recover<S: Session>(
            session: &mut S,
            stream: &str,
            scheme: crate::VersionScheme,
            entries: &[crate::CatalogEntry],
            recovery: &crate::Recovery<'_>,
        ) -> Result<(), crate::Error<S::Error>> {
            let mut guard = RunGuard { session, armed: true };
            let result = guard.session.recover(stream, scheme, entries, recovery) $($await)*;

            guard.armed = false;

            result
        }

        /// Reads the target without exclusion, abandoning the session if the
        /// call ends without returning.
        pub(crate) $($async)* fn inspect<S: Session>(
            session: &mut S,
        ) -> Result<crate::Inspection, crate::Error<S::Error>> {
            let mut guard = RunGuard { session, armed: true };
            let result = guard.session.inspect() $($await)*;

            guard.armed = false;

            result
        }

        /// Abandons the session unless disarmed: the run ended without
        /// returning, through a dropped future or an unwinding panic.
        struct RunGuard<'a, S: Session> {
            session: &'a mut S,
            armed: bool,
        }

        impl<S: Session> Drop for RunGuard<'_, S> {
            fn drop(&mut self) {
                if self.armed {
                    self.session.abandon();
                }
            }
        }

        $($async)* fn apply_locked<S: Session, M: Migration<S> $($bound)*>(
            session: &mut S,
            catalog: &crate::Catalog<'_, M>,
            options: &crate::ApplyOptions,
            report: &mut crate::ApplyReport,
        ) -> Result<(), crate::RunError<S::Error, M::Error>> {
            let history = session.history() $($await)* .map_err(crate::RunError::infrastructure)?;
            let pending = crate::runner::pending(catalog, &history, options.order)
                .map_err(crate::RunError::infrastructure)?;

            for entry in pending.entries {
                let attempt = session
                    .start_attempt(entry)
                    $($await)*
                    .map_err(crate::RunError::infrastructure)?;

                if let Err(source) = session.begin_migration(entry.metadata.policy) $($await)* {
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

                report.applied.push(entry.version);

                if pending.late.contains(&entry.version) {
                    report.late.push(entry.version);
                }
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
