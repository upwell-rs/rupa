/// The MySQL recovery steps, expanded inside a mode's steps module, whose
/// helpers and imports they use: `.await` follows each IO call in async mode.
macro_rules! recovery {
    ([$($async:tt)*] [$($await:tt)*]) => {
        /// Checks and records an assertion about an attempt in one InnoDB
        /// transaction, under the named lock.
        $($async)* fn record_recovery<T: Io<State = State>>(
            io: &mut T,
            attempt: i64,
            stream: &str,
            scheme: VersionScheme,
            entries: &[CatalogEntry],
            recovery: &Recovery<'_>,
        ) -> Result<(), Error<T::Error>> {
            if !has_journal(io) $($await)*? {
                return Err(RecoveryError::UnknownAttempt(attempt).into());
            }

            check_header(io, stream, scheme) $($await)*?;
            batch(io, "START TRANSACTION") $($await)*?;

            let rows = query(io, sql::ATTEMPT, &[Value::I64(attempt)], &ATTEMPT_COLUMNS) $($await)*?;
            let (resolved, covers) = resolve(attempt, recovery.action, rows, entries)?;
            let count = execute(io, sql::FINISH, &finish(attempt, resolved, Some(recovery))) $($await)*?;

            if count != 1 {
                return Err(Error::History(
                    "attempt resolution did not match a running attempt".into(),
                ));
            }

            record_covered(io, covers, attempt) $($await)*?;
            batch(io, "COMMIT") $($await)*
        }

        /// Records an operator's assertion about an attempt under the named
        /// lock, then releases it. MySQL sessions hold no durable claim.
        pub $($async)* fn recover<T: Io<State = State>>(
            io: &mut T,
            stream: &str,
            scheme: VersionScheme,
            entries: &[CatalogEntry],
            recovery: &Recovery<'_>,
        ) -> Result<(), Error<T::Error>> {
            check_reason(recovery)?;
            io.state().idle()?;

            let (RecoveryAction::Completed(attempt) | RecoveryAction::Restored(attempt)) =
                recovery.action
            else {
                return Err(RecoveryError::NoClaim.into());
            };
            let connection = describe_connection(io) $($await)*?;
            let lock_name = connection.lock();

            lock(io, &lock_name) $($await)*?;

            let recorded = record_recovery(io, attempt, stream, scheme, entries, recovery) $($await)*;

            if let Err(error) = recorded {
                if io.batch("ROLLBACK") $($await)* .is_err() {
                    io.state().poisoned = true;
                }

                release_after_failure(io, &lock_name, &connection.sql_mode) $($await)*;

                return Err(error);
            }

            io.state().owner = Some(Owner {
                lock: lock_name,
                sql_mode: connection.sql_mode,
            });

            release(io) $($await)*
        }
    };
}

pub(super) use recovery;
