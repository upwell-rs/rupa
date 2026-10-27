//! The clap command line for the migration commands.
//!
//! [`Migrate`] is a complete parser for a program of its own, and an
//! `Args` to embed in an application's command line, such as a `migrate`
//! subcommand or one per stream. `sync::execute` or `asynchronous::execute`
//! runs a parsed one, prints its outcome and returns its [`Exit`](super::Exit) as an
//! `ExitCode`; `main` also parses the process arguments.

mod args;

use std::io::Write as _;
use std::process::ExitCode;

use super::{Format, Outcome, Redactor};

pub use args::{
    Action, AdoptArgs, ApplyArgs, Audit, CreateArgs, Migrate, Operation, RecoverArgs, RevertArgs,
};

/// Prints `outcome` in `format`, masking what `redactor` knows: a
/// successful human outcome to stdout and any other to stderr, JSON always
/// to stdout. Returns its exit status.
pub fn print(outcome: &Outcome<'_>, format: Format, redactor: &Redactor) -> ExitCode {
    let exit = outcome.exit();
    let text = outcome.render(format, redactor);
    let written = match (format, exit) {
        (Format::Human, super::Exit::Success) | (Format::Json, _) => {
            writeln!(std::io::stdout().lock(), "{}", text.trim_end())
        }
        (Format::Human, _) => writeln!(std::io::stderr().lock(), "{}", text.trim_end()),
    };

    match written {
        Ok(()) => exit.into(),
        Err(_) => super::Exit::Failed.into(),
    }
}

/// Emits one mode's `execute` and `main`.
macro_rules! entry {
    ($mode:ident [$($async:tt)*] [$($await:tt)*] [$($bound:tt)*] [$($connect:tt)*]) => {
        pub mod $mode {
            //! The command line on this mode's sessions.

            use std::process::ExitCode;

            use clap::Parser as _;

            use super::{Migrate, print};
            use crate::$mode::{Migration, Session};
            use crate::Catalog;
            use crate::commands::{Exit, Failure, Outcome, redact_url};

            /// Runs `migrate` against `catalog` and prints its outcome.
            /// `connect` receives `--url` (or `DATABASE_URL`), and is called
            /// only by a command that reads or changes the target.
            pub $($async)* fn execute<M, S, E, $($connect)*>(
                migrate: &Migrate,
                catalog: &Catalog<'_, M>,
                connect: F,
            ) -> ExitCode
            where
                M: Migration<S> $($bound)*,
                S: Session,
                E: std::error::Error + 'static,
            {
                let url = migrate.url.as_deref();
                let mut outcome = match migrate.command(catalog.scheme()) {
                    Ok(command) => {
                        crate::commands::$mode::run(catalog, &command, || connect(url)) $($await)*
                    }
                    Err(error) => Outcome::new(migrate.operation.name(), catalog)
                        .fail(Failure::new(&error, Exit::Usage)),
                };

                outcome.target = url.map(redact_url);

                print(&outcome, migrate.format, &migrate.redactor())
            }

            /// Parses the process arguments as [`Migrate`], then runs it
            /// like [`execute`]. Invalid arguments exit with status `2`.
            pub $($async)* fn main<M, S, E, $($connect)*>(
                catalog: &Catalog<'_, M>,
                connect: F,
            ) -> ExitCode
            where
                M: Migration<S> $($bound)*,
                S: Session,
                E: std::error::Error + 'static,
            {
                execute(&Migrate::parse(), catalog, connect) $($await)*
            }
        }
    };
}

#[cfg(feature = "sync")]
entry!(sync [] [] [] [F: FnOnce(Option<&str>) -> Result<S, E>]);

#[cfg(feature = "async")]
entry!(asynchronous [async] [.await] [+ Send + Sync]
    [F: FnOnce(Option<&str>) -> C, C: std::future::Future<Output = Result<S, E>>]);
