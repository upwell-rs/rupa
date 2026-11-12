use std::path::PathBuf;

use clap::builder::PossibleValue;
use clap::{ArgGroup, Args, Parser, Subcommand, ValueEnum};

use crate::commands::{Authoring, Command, Create, Format, Redactor};
use crate::{
    Adoption, ApplyOptions, Engine, Init, MigrationKind, Order, Recovery, RecoveryAction,
    RevertTarget, VersionError, VersionScheme,
};

/// Inspect and explicitly change one catalog's migration target.
#[derive(Debug, Clone, Parser)]
#[command(about = "Inspect and explicitly change a migration target")]
pub struct Migrate {
    /// The command to run.
    #[command(subcommand)]
    pub operation: Operation,
    /// How the outcome is printed.
    #[arg(long, value_enum, default_value_t, global = true)]
    pub format: Format,
    /// The target's connection string. Its passwords never appear in output.
    #[arg(long, env = "DATABASE_URL", hide_env_values = true, global = true)]
    pub url: Option<String>,
}

/// One migration command.
#[derive(Debug, Clone, Subcommand)]
pub enum Operation {
    /// Write a new migration source, offline.
    Create(CreateArgs),
    /// Check the catalog offline against what an engine executes.
    Validate {
        /// The engine.
        #[arg(long, value_enum)]
        engine: Engine,
    },
    /// Read the target against the catalog. Writes nothing and runs no body.
    Status,
    /// Show what apply would run. Writes nothing and runs no body.
    Plan(ApplyArgs),
    /// Apply pending migrations.
    Apply(ApplyArgs),
    /// Undo the newest installations, or with --dry-run show what would be undone.
    Revert(RevertArgs),
    /// Record that an unmanaged target already holds the catalog through a version.
    Adopt(AdoptArgs),
    /// Record an operator's assertion about an interrupted run.
    Recover(RecoverArgs),
}

/// Arguments of `create`.
#[derive(Debug, Clone, Args)]
pub struct CreateArgs {
    /// The snake_case name after the version.
    pub name: String,
    /// The migration directory.
    #[arg(long, env = "RUPA_MIGRATIONS_DIR", default_value = "migrations")]
    pub dir: PathBuf,
    /// How the body is written.
    #[arg(long = "as", value_enum, default_value_t = Authoring::Sql)]
    pub authoring: Authoring,
    /// Write a snapshot of the newest incremental instead.
    #[arg(long)]
    pub snapshot: bool,
    /// Also write a down body.
    #[arg(long, conflicts_with = "snapshot")]
    pub down: bool,
}

/// Arguments of `plan` and `apply`.
#[derive(Debug, Clone, Args)]
pub struct ApplyArgs {
    /// Apply up to and including this version.
    #[arg(long)]
    pub through: Option<String>,
    /// Run pending versions below an installed one, after it.
    #[arg(long)]
    pub allow_late: bool,
    /// On a fresh target, replay every incremental instead of a snapshot.
    #[arg(long)]
    pub full_replay: bool,
}

/// Arguments of `revert`.
#[derive(Debug, Clone, Args)]
#[command(group(ArgGroup::new("selection").required(true).args(["to", "last"])))]
pub struct RevertArgs {
    /// Undo every installation above this version.
    #[arg(long)]
    pub to: Option<String>,
    /// Undo this many of the newest installations.
    #[arg(long)]
    pub last: Option<usize>,
    /// Show what would be undone. Writes nothing and runs no body.
    #[arg(long)]
    pub dry_run: bool,
}

/// Arguments of `adopt`.
#[derive(Debug, Clone, Args)]
pub struct AdoptArgs {
    /// The newest version the target already holds.
    #[arg(long)]
    pub through: String,
    /// The audit trail of the assertion.
    #[command(flatten)]
    pub audit: Audit,
}

/// Arguments of `recover`.
#[derive(Debug, Clone, Args)]
pub struct RecoverArgs {
    /// What the operator asserts.
    #[command(subcommand)]
    pub action: Action,
}

/// What an operator asserts about an interrupted run.
#[derive(Debug, Clone, Subcommand)]
pub enum Action {
    /// The attempt's effects are complete: record it as installed.
    Completed {
        /// The attempt id status reports.
        attempt: i64,
        /// The audit trail of the assertion.
        #[command(flatten)]
        audit: Audit,
    },
    /// The attempt's effects were manually undone: abandon it.
    Restored {
        /// The attempt id status reports.
        attempt: i64,
        /// The audit trail of the assertion.
        #[command(flatten)]
        audit: Audit,
    },
    /// The runner that left SQLite's owner claim has stopped: clear it.
    ReleaseClaim {
        /// The audit trail of the assertion.
        #[command(flatten)]
        audit: Audit,
    },
}

/// Why an operator's assertion holds, recorded in the journal.
#[derive(Debug, Clone, Args)]
pub struct Audit {
    /// Why the assertion holds.
    #[arg(long)]
    pub reason: String,
    /// Supporting evidence, such as a ticket, log or backup reference.
    #[arg(long)]
    pub evidence: Option<String>,
}

impl Migrate {
    /// The typed command, its versions parsed with `scheme`.
    pub fn command(&self, scheme: VersionScheme) -> Result<Command<'_>, VersionError> {
        let command = match &self.operation {
            Operation::Create(args) => Command::Create(Create {
                dir: &args.dir,
                name: &args.name,
                kind: if args.snapshot {
                    MigrationKind::Snapshot
                } else {
                    MigrationKind::Incremental
                },
                authoring: args.authoring,
                down: args.down,
            }),
            Operation::Validate { engine } => Command::Validate(*engine),
            Operation::Status => Command::Status,
            Operation::Plan(args) => Command::Plan(args.options(scheme)?),
            Operation::Apply(args) => Command::Apply(args.options(scheme)?),
            Operation::Revert(args) => {
                let target = match (&args.to, args.last) {
                    (Some(to), _) => RevertTarget::To(scheme.parse(to)?),
                    (None, last) => RevertTarget::Last(last.unwrap_or_default()),
                };

                if args.dry_run {
                    Command::RevertPlan(target)
                } else {
                    Command::Revert(target)
                }
            }
            Operation::Adopt(args) => Command::Adopt(Adoption {
                through: scheme.parse(&args.through)?,
                reason: &args.audit.reason,
                evidence: args.audit.evidence.as_deref(),
            }),
            Operation::Recover(args) => {
                let (action, audit) = match &args.action {
                    Action::Completed { attempt, audit } => {
                        (RecoveryAction::Completed(*attempt), audit)
                    }
                    Action::Restored { attempt, audit } => {
                        (RecoveryAction::Restored(*attempt), audit)
                    }
                    Action::ReleaseClaim { audit } => (RecoveryAction::ReleaseClaim, audit),
                };

                Command::Recover(Recovery {
                    action,
                    reason: &audit.reason,
                    evidence: audit.evidence.as_deref(),
                })
            }
        };

        Ok(command)
    }

    /// A redactor masking the passwords of `--url`.
    pub fn redactor(&self) -> Redactor {
        match &self.url {
            Some(url) => Redactor::new().url(url),
            None => Redactor::new(),
        }
    }
}

impl Operation {
    /// The command's stable name, which reports carry.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Create(_) => "create",
            Self::Validate { .. } => "validate",
            Self::Status => "status",
            Self::Plan(_) => "plan",
            Self::Apply(_) => "apply",
            Self::Revert(args) if args.dry_run => "revert-plan",
            Self::Revert(_) => "revert",
            Self::Adopt(_) => "adopt",
            Self::Recover(_) => "recover",
        }
    }
}

impl ApplyArgs {
    /// The apply options these arguments select.
    pub fn options(&self, scheme: VersionScheme) -> Result<ApplyOptions, VersionError> {
        let through = self
            .through
            .as_deref()
            .map(|v| scheme.parse(v))
            .transpose()?;

        Ok(ApplyOptions {
            order: if self.allow_late {
                Order::AllowLate
            } else {
                Order::Strict
            },
            through,
            init: if self.full_replay {
                Init::FullReplay
            } else {
                Init::PreferSnapshot
            },
        })
    }
}

/// Spells each variant of a value enum with its `PossibleValue` name.
macro_rules! value_enum {
    ($type:ty: $($variant:ident => $name:literal),+ $(,)?) => {
        impl ValueEnum for $type {
            fn value_variants<'a>() -> &'a [Self] {
                &[$(<$type>::$variant),+]
            }

            fn to_possible_value(&self) -> Option<PossibleValue> {
                Some(PossibleValue::new(match self {
                    $(Self::$variant => $name),+
                }))
            }
        }
    };
}

value_enum!(Format: Human => "human", Json => "json");
value_enum!(Engine: Postgres => "postgres", MySql => "mysql", Sqlite => "sqlite");
value_enum!(Authoring: Sql => "sql", Planned => "planned", Sync => "sync", Async => "async");
