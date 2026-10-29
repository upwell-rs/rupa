/// The MySQL coverage steps: reading the coverage claim, recording
/// covered incrementals, and adoption. Expanded inside a mode's steps
/// module, whose helpers and imports they use: `.await` follows each IO call
/// in async mode.
macro_rules! coverage_steps {
    ([$($async:tt)*] [$($await:tt)*]) => {
        /// Whether the journal in the selected database records a completed
        /// installation or coverage claim, undone ones included.
        pub $($async)* fn initialized<T: Io<State = State>>(io: &mut T) -> Result<bool, Error<T::Error>> {
            io.clean() $($await)* .map_err(Error::Database)?;

            if !has_journal(io) $($await)*? {
                return Ok(false);
            }

            let count = scalar(io, sql::INITIALIZED, &[], SqlType::I64) $($await)*?;

            Ok(int(&count).is_some_and(|count| count > 0))
        }

        /// Reads the coverage claim from the selected database.
        pub $($async)* fn coverage<T: Io<State = State>>(
            io: &mut T,
        ) -> Result<Option<Coverage>, Error<T::Error>> {
            io.clean() $($await)* .map_err(Error::Database)?;

            if !has_journal(io) $($await)*? {
                return Ok(None);
            }

            let rows = query(io, sql::COVERAGE, &[], &COVERAGE_COLUMNS) $($await)*?;

            decode_coverage(rows)
        }

        /// Records `members` as covered by snapshot attempt or adoption
        /// event `source`.
        $($async)* fn record_covered<T: Io<State = State>>(
            io: &mut T,
            members: &[Covered],
            source: i64,
        ) -> Result<(), Error<T::Error>> {
            for member in members {
                execute(io, sql::INSERT_COVERED, &covered_row(member, source)) $($await)*?;
            }

            Ok(())
        }

        /// Creates the journal of an unmanaged database, then records its
        /// header, the adoption event and its `members` in one InnoDB
        /// transaction under the named lock. MySQL commits `CREATE TABLE`
        /// implicitly: an interrupted adoption leaves a journal without a
        /// header, which every later run refuses as invalid.
        $($async)* fn record_adoption<T: Io<State = State>>(
            io: &mut T,
            stream: &str,
            scheme: VersionScheme,
            adoption: &Adoption<'_>,
            members: &[Covered],
        ) -> Result<(), Error<T::Error>> {
            if has_journal(io) $($await)*? {
                return Err(AdoptionError::Managed.into());
            }

            let objects = scalar(io, sql::USER_OBJECTS, &[], SqlType::I64) $($await)*?;

            check_unmanaged(&objects)?;
            batch(io, sql::CREATE) $($await)*?;
            batch(io, "START TRANSACTION") $($await)*?;
            execute(
                io,
                sql::INSERT_HEADER,
                &[bound(stream), bound(scheme.as_str()), bound(CHECKSUM_FORMAT)],
            ) $($await)*?;
            execute(io, sql::INSERT_ADOPTION, &adoption_row(adoption)) $($await)*?;

            let id = scalar(io, "SELECT LAST_INSERT_ID()", &[], SqlType::I64) $($await)*?;
            let Some(id) = int(&id) else {
                return Err(Error::History("invalid adoption id".into()));
            };

            record_covered(io, members, id) $($await)*?;
            batch(io, "COMMIT") $($await)*?;
            validate_schema(io) $($await)*
        }

        /// Records an operator's `adoption` of an unmanaged target under the
        /// named lock, then releases it.
        pub $($async)* fn adopt<T: Io<State = State>>(
            io: &mut T,
            stream: &str,
            scheme: VersionScheme,
            adoption: &Adoption<'_>,
            members: &[Covered],
        ) -> Result<(), Error<T::Error>> {
            io.state().idle()?;

            let connection = describe_connection(io) $($await)*?;
            let lock_name = connection.lock();

            lock(io, &lock_name) $($await)*?;

            let recorded = record_adoption(io, stream, scheme, adoption, members) $($await)*;

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

pub(super) use coverage_steps;
