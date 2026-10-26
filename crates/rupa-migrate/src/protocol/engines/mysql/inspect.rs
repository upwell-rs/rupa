/// The MySQL inspection steps, expanded inside a mode's steps module, whose
/// helpers and imports they use: `.await` follows each IO call in async mode.
macro_rules! inspect {
    ([$($async:tt)*] [$($await:tt)*]) => {
        /// Reads the named lock and the journal inside the snapshot
        /// [`inspect`] opened.
        $($async)* fn observe<T: Io<State = State>>(
            io: &mut T,
            connection: &Connection,
        ) -> Result<Inspection, Error<T::Error>> {
            let locked = scalar(io, sql::LOCKED, &[bound(&connection.lock())], SqlType::I64) $($await)*?;
            let exclusion = if int(&locked) == Some(1) {
                Exclusion::Locked
            } else {
                Exclusion::Free
            };

            if !has_journal(io) $($await)*? {
                let objects = scalar(io, sql::USER_OBJECTS, &[], SqlType::I64) $($await)*?;

                return Ok(Inspection { exclusion, journal: absent(&objects) });
            }

            let stored = query(io, sql::STORED, &[], &[SqlType::Text; 2]) $($await)*?;
            let history = query(io, sql::HISTORY, &[], &HISTORY_COLUMNS) $($await)*?;
            let coverage = query(io, sql::COVERAGE, &[], &COVERAGE_COLUMNS) $($await)*?;
            let running = query(io, sql::RUNNING, &[], &RUNNING_COLUMNS) $($await)*?;
            let [stream, scheme] = <[Value; 2]>::try_from(single(stored, "journal header")?)
                .map_err(|_| Error::History("invalid journal header".into()))?;

            Ok(Inspection {
                exclusion,
                journal: records(Rows {
                    stream,
                    scheme,
                    history,
                    coverage,
                    running,
                })?,
            })
        }

        /// Reads the journal and whether another session holds the named
        /// lock, in one read-only snapshot, without taking the lock. A
        /// snapshot that cannot be ended poisons the session.
        pub $($async)* fn inspect<T: Io<State = State>>(io: &mut T) -> Result<Inspection, Error<T::Error>> {
            io.state().idle()?;

            let connection = describe_connection(io) $($await)*?;

            batch(io, sql::READ_ONLY) $($await)*?;

            let observed = observe(io, &connection) $($await)*;

            if let Err(error) = io.batch("ROLLBACK") $($await)* {
                io.state().poisoned = true;

                return observed.and(Err(Error::Database(error)));
            }

            observed
        }
    };
}

pub(super) use inspect;
