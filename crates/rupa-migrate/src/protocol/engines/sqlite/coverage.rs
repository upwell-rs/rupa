/// The SQLite coverage steps: reading the coverage claim, recording
/// covered incrementals, and adoption. Expanded inside a mode's steps
/// module, whose helpers and imports they use: `.await` follows each IO call
/// in async mode.
macro_rules! coverage_steps {
    ([$($async:tt)*] [$($await:tt)*]) => {
        /// Whether the journal in `main` records a completed installation or
        /// coverage claim, undone ones included.
        pub $($async)* fn initialized<T: SqliteIo<State = State>>(io: &mut T) -> Result<bool, Error<T::Error>> {
            if !has_journal(io) $($await)*? {
                return Ok(false);
            }

            let count = scalar(io, sql::INITIALIZED, &[], SqlType::I64) $($await)*?;

            Ok(int(&count).is_some_and(|count| count > 0))
        }

        /// Reads the coverage claim from `main`.
        pub $($async)* fn coverage<T: SqliteIo<State = State>>(
            io: &mut T,
        ) -> Result<Option<Coverage>, Error<T::Error>> {
            if !has_journal(io) $($await)*? {
                return Ok(None);
            }

            let rows = query(io, sql::COVERAGE, &[], &COVERAGE_COLUMNS) $($await)*?;

            decode_coverage(rows)
        }

        /// Records `members` covered by snapshot attempt or adoption event `source`.
        $($async)* fn record_covered<T: SqliteIo<State = State>>(
            io: &mut T,
            members: &[Covered],
            source: i64,
        ) -> Result<(), Error<T::Error>> {
            for member in members {
                execute(io, sql::INSERT_COVERED, &covered_row(member, source)) $($await)*?;
            }

            Ok(())
        }

        /// Creates the journal of an unmanaged `main` with the adoption
        /// event and its `members`, inside the writer transaction.
        $($async)* fn record_adoption<T: SqliteIo<State = State>>(
            io: &mut T,
            stream: &str,
            scheme: VersionScheme,
            adoption: &crate::Adoption<'_>,
            members: &[crate::Covered],
        ) -> Result<(), Error<T::Error>> {
            if has_journal(io) $($await)*? {
                return Err(crate::AdoptionError::Managed.into());
            }

            let objects = scalar(io, sql::USER_OBJECTS, &[], SqlType::I64) $($await)*?;

            crate::protocol::check_unmanaged(&objects)?;
            create_journal(io, stream, scheme) $($await)*?;
            execute(io, sql::INSERT_ADOPTION, &crate::protocol::adoption_row(adoption)) $($await)*?;

            let id = scalar(io, "SELECT last_insert_rowid()", &[], SqlType::I64) $($await)*?;
            let Some(id) = int(&id) else {
                return Err(Error::History("invalid adoption id".into()));
            };

            record_covered(io, members, id) $($await)*
        }

        /// Records an operator's `adoption` of an unmanaged target in one
        /// writer transaction, restoring the connection's `busy_timeout` after.
        pub $($async)* fn adopt<T: SqliteIo<State = State>>(
            io: &mut T,
            stream: &str,
            scheme: VersionScheme,
            adoption: &crate::Adoption<'_>,
            members: &[crate::Covered],
        ) -> Result<(), Error<T::Error>> {
            let original = begin_writer(io) $($await)*?;
            let recorded = record_adoption(io, stream, scheme, adoption, members) $($await)*;

            end_writer(io, original, recorded) $($await)*
        }
    };
}

pub(super) use coverage_steps;
