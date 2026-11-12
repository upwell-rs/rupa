use std::error::Error as StdError;

use super::{CreateError, Created, Exit, Redactor, human, json};
use crate::{
    AdoptReport, ApplyPlan, ApplyReport, Blocker, Catalog, Engine, Error, RecoveryAction,
    RevertPlan, RevertReport, RunError, Status, ValidationError, Version, VersionScheme,
};

/// How an outcome is rendered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Format {
    /// Lines for people.
    #[default]
    Human,
    /// One JSON document, `rupa-migrate-report/1`.
    Json,
}

/// What a command found or did.
#[derive(Debug)]
pub enum Report<'c> {
    /// `create` wrote these files.
    Created(Created),
    /// The catalog passes `validate` for the engine.
    Validated(Engine),
    /// `status` read the target.
    Status(Status<'c>),
    /// What an apply would run.
    Plan(ApplyPlan<'c>),
    /// What a revert would undo.
    RevertPlan(RevertPlan<'c>),
    /// What an apply committed, also when it failed partway.
    Applied(ApplyReport),
    /// What a revert committed, also when it failed partway.
    Reverted(RevertReport),
    /// What an adoption recorded, and the assertion it rests on.
    Adopted {
        /// The incrementals recorded as covered.
        report: AdoptReport,
        /// The boundary the operator asserted.
        through: Version,
        /// The audit reason.
        reason: String,
        /// The audit evidence.
        evidence: Option<String>,
    },
    /// The recovery assertion recorded.
    Recovered {
        /// What the operator asserted.
        action: RecoveryAction,
        /// The audit reason.
        reason: String,
        /// The audit evidence.
        evidence: Option<String>,
    },
}

/// Why a command did not succeed, with its causes and source location.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Failure {
    /// The exit status it ends the command with.
    pub exit: Exit,
    /// The error, as displayed.
    pub message: String,
    /// The error's sources, outermost first.
    pub causes: Vec<String>,
    /// The migration whose body failed, if one did.
    pub migration: Option<Version>,
    /// That migration's source file, as the catalog names it.
    pub source: Option<&'static str>,
    /// What refused the command, each with its artifact identity.
    pub blockers: Vec<Blocker>,
    /// Rollback or release failures after the error.
    pub cleanup: Vec<String>,
}

impl Failure {
    /// `error` and its source chain, ending a command with `exit`.
    pub fn new(error: &(dyn StdError + 'static), exit: Exit) -> Self {
        Self {
            exit,
            message: error.to_string(),
            causes: causes(error.source()),
            migration: None,
            source: None,
            blockers: Vec::new(),
            cleanup: Vec::new(),
        }
    }

    /// An infrastructure `error`, with its exit status and blockers.
    pub fn infrastructure<E: StdError + 'static>(error: Error<E>) -> Self {
        let exit = Exit::of(&error);
        let mut failure = Self::new(&error, exit);

        if let Error::Blocked(blockers) = error {
            failure.blockers = blockers;
        }

        failure
    }

    fn cleaned<E: StdError + 'static>(mut self, cleanup: Vec<Error<E>>) -> Self {
        self.cleanup = cleanup.iter().map(ToString::to_string).collect();

        self
    }
}

/// A command's result: its report, its failure, or a partial report and
/// the failure that ended it.
#[derive(Debug)]
pub struct Outcome<'c> {
    /// The command's name.
    pub command: &'static str,
    /// The catalog's stream.
    pub stream: &'c str,
    /// The catalog's version scheme.
    pub scheme: VersionScheme,
    /// The target, as the caller shows it; never with credentials.
    pub target: Option<String>,
    /// What the command found or did.
    pub report: Option<Report<'c>>,
    /// Why it did not succeed.
    pub failure: Option<Failure>,
}

impl<'c> Outcome<'c> {
    /// An outcome of `command` on `catalog` that has nothing to report yet.
    pub fn new<M>(command: &'static str, catalog: &Catalog<'c, M>) -> Self {
        Self {
            command,
            stream: catalog.stream(),
            scheme: catalog.scheme(),
            target: None,
            report: None,
            failure: None,
        }
    }

    /// The exit status: the failure's, or blocked when a read reports blockers.
    pub fn exit(&self) -> Exit {
        let blocked = match &self.report {
            Some(Report::Status(status)) => !status.blockers.is_empty(),
            Some(Report::Plan(plan)) => !plan.status.blockers.is_empty(),
            Some(Report::RevertPlan(plan)) => !plan.status.blockers.is_empty(),
            _ => false,
        };

        match &self.failure {
            Some(failure) => failure.exit,
            None if blocked => Exit::Blocked,
            None => Exit::Success,
        }
    }

    /// The outcome in `format`, every secret `redactor` knows masked.
    pub fn render(&self, format: Format, redactor: &Redactor) -> String {
        let text = match format {
            Format::Human => human::render(self),
            Format::Json => json::render(self).to_string(),
        };

        redactor.redact(&text)
    }

    pub(super) fn report(mut self, report: Report<'c>) -> Self {
        self.report = Some(report);

        self
    }

    pub(super) fn fail(mut self, failure: Failure) -> Self {
        self.failure = Some(failure);

        self
    }

    pub(super) fn created(self, result: Result<Created, CreateError>) -> Self {
        match result {
            Ok(created) => self.report(Report::Created(created)),
            Err(error) => {
                let exit = match error {
                    CreateError::Name(_) | CreateError::SnapshotDown => Exit::Usage,
                    CreateError::Io { .. } | CreateError::Clock(_) => Exit::Failed,
                    _ => Exit::Blocked,
                };

                self.fail(Failure::new(&error, exit))
            }
        }
    }

    pub(super) fn validated(self, result: Result<(), ValidationError>, engine: Engine) -> Self {
        match result {
            Ok(()) => self.report(Report::Validated(engine)),
            Err(error) => self.fail(Failure::new(&error, Exit::Blocked)),
        }
    }

    pub(super) fn inspected<T, E: StdError + 'static>(
        self,
        result: Result<T, Error<E>>,
        report: impl FnOnce(T) -> Report<'c>,
    ) -> Self {
        match result {
            Ok(found) => self.report(report(found)),
            Err(error) => self.fail(Failure::infrastructure(error)),
        }
    }

    pub(super) fn ran<R, E, M, N>(
        self,
        catalog: &Catalog<'c, N>,
        result: Result<R, RunError<E, M, R>>,
        report: impl FnOnce(R) -> Report<'c>,
    ) -> Self
    where
        R: std::fmt::Debug,
        E: StdError + 'static,
        M: StdError + 'static,
    {
        let (committed, failure) = match result {
            Ok(committed) => return self.report(report(committed)),
            Err(RunError::Infrastructure {
                source,
                report: committed,
                cleanup,
            }) => (committed, Failure::infrastructure(source).cleaned(cleanup)),
            Err(RunError::Migration {
                version,
                source,
                report: committed,
                cleanup,
            }) => {
                let mut failure = Failure::new(&source, Exit::Migration).cleaned(cleanup);

                failure.message = format!("migration {version} failed: {}", failure.message);
                failure.source = catalog
                    .entries
                    .iter()
                    .find(|entry| entry.version == version)
                    .and_then(|entry| entry.metadata.inputs.first())
                    .map(|input| input.path);
                failure.migration = Some(version);

                (committed, failure)
            }
        };

        self.report(report(*committed)).fail(failure)
    }
}

/// The messages of `error` and its sources.
fn causes(mut error: Option<&(dyn StdError + 'static)>) -> Vec<String> {
    let mut causes = Vec::new();

    while let Some(cause) = error {
        causes.push(cause.to_string());
        error = cause.source();
    }

    causes
}
