/// The PostgreSQL coverage steps: reading the coverage claim, recording
/// covered incrementals, and adoption. Expanded inside a mode's steps
/// module, whose helpers and imports they use: `.await` follows each IO call
/// in async mode.
macro_rules! coverage_steps {
    ([$($async:tt)*] [$($await:tt)*]) => {
        /// Whether the journal in the owned or current namespace records a
        /// completed installation or coverage claim, undone ones included.
        pub $($async)* fn initialized<T: Io<State = State>>(io: &mut T) -> Result<bool, Error<T::Error>> {
            let namespace = namespace(io) $($await)*?;

            if !has_journal(io, &namespace) $($await)*? {
                return Ok(false);
            }

            let count = scalar(io, &sql::initialized(&journal(&namespace)), &[], SqlType::I64) $($await)*?;

            Ok(int(&count).is_some_and(|count| count > 0))
        }

        /// Reads the coverage claim from the owned or current namespace.
        pub $($async)* fn coverage<T: Io<State = State>>(
            io: &mut T,
        ) -> Result<Option<Coverage>, Error<T::Error>> {
            let namespace = namespace(io) $($await)*?;

            if !has_journal(io, &namespace) $($await)*? {
                return Ok(None);
            }

            let rows = query(io, &sql::coverage(&journal(&namespace)), &[], &COVERAGE_COLUMNS) $($await)*?;

            decode_coverage(rows)
        }

        /// Records `members` in `journal` as covered by snapshot attempt
        /// or adoption event `source`.
        $($async)* fn record_covered<T: Io<State = State>>(
            io: &mut T,
            journal: &str,
            members: &[Covered],
            source: i64,
        ) -> Result<(), Error<T::Error>> {
            for member in members {
                execute(io, &sql::insert_covered(journal), &covered_row(member, source)) $($await)*?;
            }

            Ok(())
        }

        /// Creates the journal of an unmanaged namespace with the adoption
        /// event and its `members`, in one transaction under the advisory lock.
        $($async)* fn record_adoption<T: Io<State = State>>(
            io: &mut T,
            namespace: &str,
            stream: &str,
            scheme: VersionScheme,
            adoption: &Adoption<'_>,
            members: &[Covered],
        ) -> Result<(), Error<T::Error>> {
            let journal = journal(namespace);

            batch(io, "BEGIN") $($await)*?;

            if has_journal(io, namespace) $($await)*? {
                return Err(AdoptionError::Managed.into());
            }

            let objects = scalar(
                io,
                sql::USER_OBJECTS,
                &[Value::Text(namespace.to_owned())],
                SqlType::I64,
            ) $($await)*?;

            check_unmanaged(&objects)?;
            create_journal(io, &journal, stream, scheme) $($await)*?;

            let rows = query(io, &sql::insert_adoption(&journal), &adoption_row(adoption), &[SqlType::I64]) $($await)*?;
            let Some(id) = rows.first().and_then(|row| int(&row[0])) else {
                return Err(Error::History("adoption insert returned no id".into()));
            };

            record_covered(io, &journal, members, id) $($await)*?;
            batch(io, "COMMIT") $($await)*
        }

        /// Records an operator's `adoption` of an unmanaged target under the
        /// advisory lock, then releases it.
        pub $($async)* fn adopt<T: Io<State = State>>(
            io: &mut T,
            stream: &str,
            scheme: VersionScheme,
            adoption: &Adoption<'_>,
            members: &[Covered],
        ) -> Result<(), Error<T::Error>> {
            io.state().idle()?;

            let connection = describe_connection(io) $($await)*?;
            let key = connection.key();

            lock(io, key) $($await)*?;

            let recorded =
                record_adoption(io, &connection.namespace, stream, scheme, adoption, members) $($await)*;

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

pub(super) use coverage_steps;
