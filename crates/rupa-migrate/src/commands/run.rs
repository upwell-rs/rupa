/// Emits one mode's `run` from a single body. Calls are suffixed with
/// `.await` in async mode only; the mode module imports the method traits
/// and `Migration` it uses.
macro_rules! commands {
    () => {
        $crate::commands::run::commands!(@emit [] [] [] [F: FnOnce() -> Result<S, E>]);
    };
    (async) => {
        $crate::commands::run::commands!(@emit [async] [.await] [+ Send + Sync]
            [F: FnOnce() -> C, C: std::future::Future<Output = Result<S, E>>]);
    };
    (@emit [$($async:tt)*] [$($await:tt)*] [$($bound:tt)*] [$($connect:tt)*]) => {
        use $crate::commands::{Command, Exit, Failure, Outcome, Report};
        use $crate::Catalog;

        /// Runs `command` against `catalog` through the engine library
        /// callers use. Connects with `connect` only for a command that
        /// reads or changes the target; `create` and `validate` stay offline.
        pub $($async)* fn run<'c, M, S, E, $($connect)*>(
            catalog: &'c Catalog<'_, M>,
            command: &Command<'_>,
            connect: F,
        ) -> Outcome<'c>
        where
            M: Migration<S> $($bound)*,
            S: Session,
            E: std::error::Error + 'static,
        {
            let outcome = Outcome::new(command.name(), catalog);
            let connection = match command {
                Command::Create(create) => {
                    return outcome.created($crate::commands::create::create(
                        catalog,
                        create,
                        std::time::SystemTime::now(),
                    ));
                }
                Command::Validate(engine) => {
                    return outcome.validated(catalog.validate(*engine), *engine);
                }
                _ => match connect() $($await)* {
                    Ok(connection) => connection,
                    Err(error) => return outcome.fail(Failure::new(&error, Exit::Failed)),
                },
            };

            match command {
                Command::Status => {
                    outcome.inspected(catalog.status(connection) $($await)*, Report::Status)
                }
                Command::Plan(options) => outcome
                    .inspected(catalog.plan_with(connection, options) $($await)*, Report::Plan),
                Command::RevertPlan(target) => outcome.inspected(
                    catalog.revert_plan(connection, *target) $($await)*,
                    Report::RevertPlan,
                ),
                Command::Apply(options) => outcome.ran(
                    catalog,
                    catalog.apply_with(connection, options) $($await)*,
                    Report::Applied,
                ),
                Command::Revert(target) => outcome.ran(
                    catalog,
                    catalog.revert(connection, *target) $($await)*,
                    Report::Reverted,
                ),
                Command::Adopt(adoption) => outcome.inspected(
                    catalog.adopt(connection, adoption) $($await)*,
                    |report| Report::Adopted {
                        report,
                        through: adoption.through,
                        reason: adoption.reason.to_owned(),
                        evidence: adoption.evidence.map(str::to_owned),
                    },
                ),
                Command::Recover(recovery) => outcome.inspected(
                    catalog.recover(connection, recovery) $($await)*,
                    |()| Report::Recovered {
                        action: recovery.action,
                        reason: recovery.reason.to_owned(),
                        evidence: recovery.evidence.map(str::to_owned),
                    },
                ),
                Command::Create(_) | Command::Validate(_) => outcome,
            }
        }
    };
}

pub(super) use commands;
