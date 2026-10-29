/// The PostgreSQL recovery steps, expanded inside a mode's steps module, whose
/// helpers and imports they use: `.await` follows each IO call in async mode.
macro_rules! recovery {
    ([$($async:tt)*] [$($await:tt)*]) => {
        /// Checks and records an assertion about an attempt, in one
        /// transaction under the advisory lock.
        $($async)* fn record_recovery<T: Io<State = State>>(
            io: &mut T,
            namespace: &str,
            stream: &str,
            scheme: VersionScheme,
            entries: &[CatalogEntry],
            recovery: &Recovery<'_>,
        ) -> Result<(), Error<T::Error>> {
            let journal = journal(namespace);
            let (RecoveryAction::Completed(attempt) | RecoveryAction::Restored(attempt)) =
                recovery.action
            else {
                return Err(RecoveryError::NoClaim.into());
            };

            batch(io, "BEGIN") $($await)*?;

            if !has_journal(io, namespace) $($await)*? {
                return Err(RecoveryError::UnknownAttempt(attempt).into());
            }

            check_header(io, &journal, stream, scheme) $($await)*?;
            reconcile(io, &journal) $($await)*?;

            let rows = query(io, &sql::attempt(&journal), &[Value::I64(attempt)], &ATTEMPT_COLUMNS) $($await)*?;
            let (resolved, covers) = resolve(attempt, recovery.action, rows, entries)?;
            let count = execute(io, &sql::finish(&journal), &finish(attempt, resolved, Some(recovery))) $($await)*?;

            if count != 1 {
                return Err(Error::History(
                    "attempt resolution did not match a running attempt".into(),
                ));
            }

            record_covered(io, &journal, covers, attempt) $($await)*?;
            batch(io, "COMMIT") $($await)*
        }

        /// Records an operator's assertion about an attempt under the
        /// advisory lock, then releases it. PostgreSQL sessions hold no
        /// durable claim to release.
        pub $($async)* fn recover<T: Io<State = State>>(
            io: &mut T,
            stream: &str,
            scheme: VersionScheme,
            entries: &[CatalogEntry],
            recovery: &Recovery<'_>,
        ) -> Result<(), Error<T::Error>> {
            check_reason(recovery)?;
            io.state().idle()?;

            if recovery.action == RecoveryAction::ReleaseClaim {
                return Err(RecoveryError::NoClaim.into());
            }

            let connection = describe_connection(io) $($await)*?;
            let key = connection.key();

            lock(io, key) $($await)*?;

            let recorded =
                record_recovery(io, &connection.namespace, stream, scheme, entries, recovery) $($await)*;

            if let Err(error) = recorded {
                unlock_after_failure(io, key) $($await)*;

                return Err(error);
            }

            io.state().owner = Some(Owner {
                key,
                namespace: connection.namespace,
            });

            release(io) $($await)*
        }
    };
}

pub(super) use recovery;
