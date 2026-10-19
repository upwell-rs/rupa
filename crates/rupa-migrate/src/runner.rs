use std::collections::BTreeSet;

use crate::{
    Catalog, Context, Error, Migration, MigrationKind, Policy, Session, ValidationError, Version,
};

/// Successfully committed versions from one explicit invocation.
#[derive(Debug, Default)]
pub struct ApplyReport {
    /// Versions executed in actual installation order.
    pub applied: Vec<Version>,
}

/// A run failed while retaining the typed body and infrastructure causes.
#[derive(Debug, thiserror::Error)]
pub enum RunError<E: std::error::Error + 'static, M: std::error::Error + 'static> {
    /// Catalog/history/session preparation or completion failed.
    #[error("{source}; cleanup errors: {cleanup:?}")]
    Infrastructure {
        /// Original infrastructure failure.
        #[source]
        source: Error<E>,
        /// Further rollback/release failures, never replacing the original error.
        cleanup: Vec<Error<E>>,
    },
    /// A user body failed; earlier migration commits remain intact.
    #[error("migration {version} failed: {source}; cleanup errors: {cleanup:?}")]
    Migration {
        /// The exact failed version.
        version: Version,
        /// Original typed application/migration error.
        #[source]
        source: M,
        /// Further rollback/release failures.
        cleanup: Vec<Error<E>>,
    },
}

impl<E: std::error::Error + 'static, M: std::error::Error + 'static> RunError<E, M> {
    fn infrastructure(source: Error<E>) -> Self {
        Self::Infrastructure {
            source,
            cleanup: Vec::new(),
        }
    }

    fn cleanup(&mut self, error: Error<E>) {
        match self {
            Self::Infrastructure { cleanup, .. } | Self::Migration { cleanup, .. } => {
                cleanup.push(error)
            }
        }
    }
}

pub(crate) fn apply<S: Session, M: Migration<S>>(
    session: &mut S,
    catalog: &Catalog<'_, M>,
) -> Result<ApplyReport, RunError<S::Error, M::Error>> {
    for entry in &catalog.entries {
        if entry.metadata.kind != MigrationKind::Incremental
            || entry.metadata.policy != Policy::TransactionalRequired
        {
            return Err(RunError::infrastructure(Error::Validation(
                ValidationError::Unsupported(entry.metadata.version.into()),
            )));
        }
    }

    session
        .acquire(catalog.stream, catalog.scheme)
        .map_err(RunError::infrastructure)?;
    let mut result = apply_locked(session, catalog);
    if let Err(cleanup) = session.release() {
        match &mut result {
            Ok(_) => result = Err(RunError::infrastructure(cleanup)),
            Err(error) => error.cleanup(cleanup),
        }
    }

    result
}

fn apply_locked<S: Session, M: Migration<S>>(
    session: &mut S,
    catalog: &Catalog<'_, M>,
) -> Result<ApplyReport, RunError<S::Error, M::Error>> {
    let history = session.history().map_err(RunError::infrastructure)?;
    let mut installed = BTreeSet::new();
    for record in &history {
        let version = catalog
            .scheme
            .parse(&record.version)
            .map_err(|e| RunError::infrastructure(Error::Validation(e.into())))?;
        let entry = catalog
            .entries
            .iter()
            .find(|e| e.version == version)
            .ok_or_else(|| {
                RunError::infrastructure(Error::History(format!(
                    "installed migration {version} is missing from the catalog"
                )))
            })?;
        if entry.checksum != record.checksum || !installed.insert(version) {
            return Err(RunError::infrastructure(Error::History(format!(
                "installed migration {version} changed checksum or appears twice"
            ))));
        }
    }

    if let Some(maximum) = installed.last() {
        for entry in &catalog.entries {
            if entry.version < *maximum && !installed.contains(&entry.version) {
                return Err(RunError::infrastructure(Error::History(format!(
                    "late migration {} below installed {maximum}; strict apply refuses it",
                    entry.version
                ))));
            }
        }
    }

    let mut report = ApplyReport::default();
    for entry in &catalog.entries {
        if installed.contains(&entry.version) {
            continue;
        }

        let attempt = session
            .start_attempt(entry)
            .map_err(RunError::infrastructure)?;
        if let Err(source) = session.begin_migration() {
            let mut error = RunError::infrastructure(source);
            if let Err(cleanup) = session.abort(attempt) {
                error.cleanup(cleanup);
            }

            return Err(error);
        }

        let mut context = Context {
            session: &mut *session,
        };
        if let Err(source) = catalog.migrations[entry.index].up(&mut context) {
            let mut error = RunError::Migration {
                version: entry.version,
                source,
                cleanup: Vec::new(),
            };
            if let Err(cleanup) = session.abort(attempt) {
                error.cleanup(cleanup);
            }

            return Err(error);
        }

        if let Err(source) = session.complete(attempt, entry) {
            let mut error = RunError::infrastructure(source);
            if let Err(cleanup) = session.abort(attempt) {
                error.cleanup(cleanup);
            }

            return Err(error);
        }
        report.applied.push(entry.version);
    }

    Ok(report)
}
