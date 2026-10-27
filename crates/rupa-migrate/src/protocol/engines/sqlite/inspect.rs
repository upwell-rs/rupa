/// The SQLite inspection steps, expanded inside a mode's steps module, whose
/// helpers and imports they use: `.await` follows each IO call in async mode.
macro_rules! inspect {
    ([$($async:tt)*] [$($await:tt)*]) => {
        /// Reads the journal and its owner claim inside the snapshot
        /// [`inspect`] opened.
        $($async)* fn observe<T: SqliteIo<State = State>>(io: &mut T) -> Result<Inspection, Error<T::Error>> {
            if !has_journal(io) $($await)*? {
                let objects = scalar(io, sql::USER_OBJECTS, &[], SqlType::I64) $($await)*?;

                return Ok(Inspection {
                    exclusion: Exclusion::Free,
                    journal: absent(&objects),
                });
            }

            let header = query(io, sql::HEADER, &[], &[SqlType::Text; 3]) $($await)*?;
            let history = query(io, sql::HISTORY, &[], &HISTORY_COLUMNS) $($await)*?;
            let coverage = query(io, sql::COVERAGE, &[], &COVERAGE_COLUMNS) $($await)*?;
            let initialized = scalar(io, sql::INITIALIZED, &[], SqlType::I64) $($await)*?;
            let running = query(io, sql::RUNNING, &[], &RUNNING_COLUMNS) $($await)*?;
            let [stream, scheme, owner] = <[Value; 3]>::try_from(single(header, "journal header")?)
                .map_err(|_| Error::History("invalid journal header".into()))?;
            let exclusion = if text(owner).is_some() {
                Exclusion::Claimed
            } else {
                Exclusion::Free
            };

            Ok(Inspection {
                exclusion,
                journal: records(Rows {
                    stream,
                    scheme,
                    history,
                    coverage,
                    initialized,
                    running,
                })?,
            })
        }

        /// Reads the journal and its owner claim in one deferred read
        /// transaction, without the writer lock. A transaction that cannot
        /// be ended poisons the session.
        pub $($async)* fn inspect<T: SqliteIo<State = State>>(io: &mut T) -> Result<Inspection, Error<T::Error>> {
            let state = io.state();

            if state.owner.is_some() || state.poisoned {
                return Err(Error::History(
                    "session already owns a run or is not clean".into(),
                ));
            }

            validate_connection(io) $($await)*?;
            batch(io, sql::READ) $($await)*?;

            let observed = observe(io) $($await)*;

            if let Err(error) = io.batch("ROLLBACK") $($await)* {
                io.state().poisoned = true;

                return observed.and(Err(Error::Database(error)));
            }

            observed
        }
    };
}

pub(super) use inspect;
