/// The SQLite recovery steps, expanded inside a mode's steps module, whose
/// helpers they use: `.await` follows each IO call in async mode.
macro_rules! recovery {
    ([$($async:tt)*] [$($await:tt)*]) => {
        /// Checks and records `recovery` inside the writer transaction. A
        /// stale claim is cleared only on the operator's assertion; an
        /// assertion about an attempt needs an unowned target, whose
        /// transactional attempts are reconciled first.
        $($async)* fn record_recovery<T: SqliteIo<State = State>>(
            io: &mut T,
            stream: &str,
            scheme: VersionScheme,
            entries: &[CatalogEntry],
            recovery: &crate::Recovery<'_>,
        ) -> Result<(), Error<T::Error>> {
            use crate::RecoveryAction;

            let action = recovery.action;

            if !has_journal(io) $($await)*? {
                return Err(match action {
                    RecoveryAction::Completed(attempt) | RecoveryAction::Restored(attempt) => {
                        crate::RecoveryError::UnknownAttempt(attempt)
                    }
                    RecoveryAction::ReleaseClaim => crate::RecoveryError::NoClaim,
                }
                .into());
            }

            let owner = check_header(io, stream, scheme) $($await)*?;
            let attempt = match (action, owner) {
                (RecoveryAction::ReleaseClaim, Some(owner)) => {
                    execute(io, sql::UNCLAIM, &[bound(&owner)]) $($await)*?;
                    execute(
                        io,
                        sql::CLAIM_RELEASED,
                        &[bound(recovery.reason), recovery.evidence.map_or(Value::Null(SqlType::Text), bound)],
                    ) $($await)*?;

                    return Ok(());
                }
                (RecoveryAction::ReleaseClaim, None) => return Err(crate::RecoveryError::NoClaim.into()),
                (_, Some(_)) => return Err(Error::Busy),
                (RecoveryAction::Completed(attempt) | RecoveryAction::Restored(attempt), None) => attempt,
            };

            reconcile(io) $($await)*?;

            let rows = query(io, sql::ATTEMPT, &[Value::I64(attempt)], &crate::protocol::ATTEMPT_COLUMNS) $($await)*?;
            let resolved = crate::protocol::resolve(attempt, action, rows, entries)?;
            let count = execute(io, sql::FINISH, &finish(attempt, resolved, Some(recovery))) $($await)*?;

            if count != 1 {
                return Err(Error::History(
                    "attempt resolution did not match a running attempt".into(),
                ));
            }

            Ok(())
        }

        /// Records an operator's `recovery` assertion in one writer
        /// transaction, restoring the connection's `busy_timeout` after.
        pub $($async)* fn recover<T: SqliteIo<State = State>>(
            io: &mut T,
            stream: &str,
            scheme: VersionScheme,
            entries: &[CatalogEntry],
            recovery: &crate::Recovery<'_>,
        ) -> Result<(), Error<T::Error>> {
            crate::protocol::check_reason(recovery)?;

            let original = begin_writer(io) $($await)*?;
            let mut recorded = record_recovery(io, stream, scheme, entries, recovery) $($await)*;
            let end = if recorded.is_ok() { "COMMIT" } else { "ROLLBACK" };

            if let Err(error) = io.batch(end) $($await)* {
                io.state().poisoned = true;
                recorded = recorded.and(Err(Error::Database(error)));
            }

            set_busy_timeout(io, original) $($await)*?;

            recorded
        }
    };
}

pub(super) use recovery;
