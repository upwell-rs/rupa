//! The execution engine shared by both modes, compiled only with one of them.

use std::fmt::Debug;

use rupa_core::dialect::DialectId;

use crate::status::Selection;
use crate::{Catalog, CatalogEntry, Error, HistoryEntry, Order, RevertTarget, RunError, Version};

impl<E: std::error::Error + 'static, M: std::error::Error + 'static, R: Default + Debug>
    RunError<E, M, R>
{
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

    fn with_report(mut self, committed: R) -> Self {
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

/// The installations a revert to `target` undoes on a target with
/// `history`, after checking history against the catalog as apply does,
/// except that late artifacts never block a revert.
pub(crate) fn undo<'c, E: std::error::Error, M>(
    catalog: &'c Catalog<'_, M>,
    history: &[HistoryEntry],
    target: RevertTarget,
) -> Result<Selection<'c>, Error<E>> {
    let mut blockers = Vec::new();

    catalog.compare(history, &mut blockers);
    Order::AllowLate.permit(&mut blockers);

    let selection = catalog.select(history, target, &mut blockers);

    if !blockers.is_empty() {
        return Err(Error::Blocked(blockers));
    }

    Ok(selection)
}

/// Combines a locked run with its release, never letting a release failure
/// replace the run's own error.
pub(crate) fn finish<
    E: std::error::Error + 'static,
    M: std::error::Error + 'static,
    R: Default + Debug,
>(
    result: Result<(), RunError<E, M, R>>,
    released: Result<(), Error<E>>,
    report: R,
) -> Result<R, RunError<E, M, R>> {
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

            /// Runs the checksummed plan of the running direction: the
            /// entry's plan for `up`, its down plan for `down`.
            $($async)* fn run_plan(&mut self) -> Result<(), crate::Error<S::Error>> {
                let Some(plan) = self.plan else {
                    return Err(crate::Error::History(
                        "a planned migration ran without its catalog plan".into(),
                    ));
                };

                for operation in plan.operations() {
                    match operation {
                        crate::Operation::Sql(sql) => self.script(sql) $($await)* ?,
                        crate::Operation::Schema(op) => self.schema(op) $($await)* ?,
                    }
                }

                Ok(())
            }
        }

        /// Executes the plans the catalog checksummed when the entry was
        /// built, not fresh calls to `plan()` or `down()`, so what runs is
        /// exactly what was verified.
        impl<S: Session, T: crate::PlannedMigration $($bound)*> Migration<S> for crate::Planned<T> {
            type Error = crate::Error<S::Error>;

            $($async)* fn up(&self, context: &mut Context<'_, S>) -> Result<(), Self::Error> {
                context.run_plan() $($await)*
            }

            $($async)* fn down(&self, context: &mut Context<'_, S>) -> Result<(), Self::Error> {
                context.run_plan() $($await)*
            }
        }

        mode!(@run [$($async)*] [$($await)*] [$($bound)*]
            apply, apply_locked, crate::ApplyReport, &crate::ApplyOptions);
        mode!(@run [$($async)*] [$($await)*] [$($bound)*]
            revert, revert_locked, crate::RevertReport, crate::RevertTarget);

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
                attempt(session, catalog, entry, crate::Direction::Up) $($await)*?;
                report.applied.push(entry.version);

                if pending.late.contains(&entry.version) {
                    report.late.push(entry.version);
                }
            }

            Ok(())
        }

        /// Undoes the installations `target` selects, newest first, each in
        /// its own attempt. `report.remaining` starts as the installed stack
        /// and loses its newest version with each completed undo.
        $($async)* fn revert_locked<S: Session, M: Migration<S> $($bound)*>(
            session: &mut S,
            catalog: &crate::Catalog<'_, M>,
            target: crate::RevertTarget,
            report: &mut crate::RevertReport,
        ) -> Result<(), crate::RunError<S::Error, M::Error, crate::RevertReport>> {
            let history = session.history() $($await)* .map_err(crate::RunError::infrastructure)?;
            let selection = crate::runner::undo(catalog, &history, target)
                .map_err(crate::RunError::infrastructure)?;

            report.remaining = selection.remaining;
            report
                .remaining
                .extend(selection.undo.iter().rev().map(|(_, entry)| entry.version));

            for (undoes, entry) in selection.undo {
                attempt(session, catalog, entry, crate::Direction::Down { undoes }) $($await)*?;
                report.remaining.pop();
                report.reverted.push(entry.version);
            }

            Ok(())
        }

        /// Runs one journaled attempt of `entry` in `direction`: its body
        /// under the entry's policy, committed with its completion.
        $($async)* fn attempt<
            S: Session,
            M: Migration<S> $($bound)*,
            R: Default + std::fmt::Debug + Send,
        >(
            session: &mut S,
            catalog: &crate::Catalog<'_, M>,
            entry: &crate::CatalogEntry,
            direction: crate::Direction,
        ) -> Result<(), crate::RunError<S::Error, M::Error, R>> {
            let attempt = session
                .start_attempt(entry, direction)
                $($await)*
                .map_err(crate::RunError::infrastructure)?;

            if let Err(source) = session.begin_migration(entry.metadata.policy) $($await)* {
                return Err(abort(session, attempt, crate::RunError::infrastructure(source)) $($await)*);
            }

            let migration = &catalog.migrations[entry.index];
            let mut context = Context {
                session: &mut *session,
                plan: None,
                missing_down: false,
            };
            let ran = match direction {
                crate::Direction::Up => {
                    context.plan = entry.plan.as_ref();

                    migration.up(&mut context) $($await)*
                }
                crate::Direction::Down { .. } => {
                    context.plan = entry.down();

                    migration.down(&mut context) $($await)*
                }
            };
            let missing = context.missing_down;

            if let Err(source) = ran {
                return Err(
                    abort(session, attempt, crate::RunError::migration(entry.version, source)) $($await)*
                );
            }

            if missing {
                let source = crate::Error::MissingDown(entry.version);

                return Err(abort(session, attempt, crate::RunError::infrastructure(source)) $($await)*);
            }

            if let Err(source) = session.complete(attempt, entry) $($await)* {
                return Err(abort(session, attempt, crate::RunError::infrastructure(source)) $($await)*);
            }

            Ok(())
        }

        /// Rolls back an unsuccessful attempt, keeping cleanup failures beside the original error.
        $($async)* fn abort<
            S: Session,
            M: std::error::Error + Send + 'static,
            R: Default + std::fmt::Debug + Send,
        >(
            session: &mut S,
            attempt: i64,
            mut error: crate::RunError<S::Error, M, R>,
        ) -> crate::RunError<S::Error, M, R> {
            if let Err(cleanup) = session.abort(attempt) $($await)* {
                error.cleanup(cleanup);
            }

            error
        }
    };
    (@run [$($async:tt)*] [$($await:tt)*] [$($bound:tt)*]
        $name:ident, $locked:ident, $report:ty, $request:ty) => {
        /// Validates the catalog for the session, then runs the request under
        /// exclusion, which it always releases. Interrupted transactional
        /// attempts found on acquisition stop the run before anything runs.
        pub(crate) $($async)* fn $name<S: Session, M: Migration<S> $($bound)*>(
            session: &mut S,
            catalog: &crate::Catalog<'_, M>,
            request: $request,
        ) -> Result<$report, crate::RunError<S::Error, M::Error, $report>> {
            let mut report = <$report>::default();

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
                $locked(&mut *guard.session, catalog, request, &mut report) $($await)*
            } else {
                Err(crate::RunError::infrastructure(crate::Error::Reconciled(reconciled)))
            };
            let released = guard.session.release() $($await)*;

            guard.armed = false;

            crate::runner::finish(result, released, report)
        }
    };
}

pub(crate) use mode;
