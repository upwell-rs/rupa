/// The MySQL session steps for one mode, as module `$mode` over that mode's
/// `Io`: `.await` follows each IO call in async mode.
macro_rules! steps {
    ($mode:ident, [$($async:tt)*] [$($await:tt)*]) => {
        /// The MySQL session steps in this mode.
        pub mod $mode {
            use rupa_core::{DialectId, SqlType, Value};

            use super::{Connection, Owner, State, bound, finish, lock_failed, sql};
            use crate::protocol::$mode::Io;
            use crate::protocol::{
                ATTEMPT_COLUMNS, HISTORY_COLUMNS, RUNNING_COLUMNS, SqlState as _, absent,
                check_reason, decode_history, int, policy, records, resolve, single, status, text,
            };
            use crate::{
                CHECKSUM_FORMAT, CatalogEntry, Error, Exclusion, HistoryEntry, Inspection, Policy,
                Recovery, RecoveryAction, RecoveryError, VersionScheme,
            };

            $($async)* fn query<T: Io<State = State>>(
                io: &mut T,
                sql: &str,
                params: &[Value],
                columns: &[SqlType],
            ) -> Result<Vec<Vec<Value>>, Error<T::Error>> {
                io.query(sql, params, columns) $($await)* .map_err(Error::Database)
            }

            /// The first column of the single row `sql` returns.
            $($async)* fn scalar<T: Io<State = State>>(
                io: &mut T,
                sql: &str,
                params: &[Value],
                column: SqlType,
            ) -> Result<Value, Error<T::Error>> {
                let rows = query(io, sql, params, &[column]) $($await)*?;

                Ok(single(rows, "control query")?.swap_remove(0))
            }

            $($async)* fn execute<T: Io<State = State>>(
                io: &mut T,
                sql: &str,
                params: &[Value],
            ) -> Result<u64, Error<T::Error>> {
                io.execute(sql, params) $($await)* .map_err(Error::Database)
            }

            $($async)* fn batch<T: Io<State = State>>(io: &mut T, sql: &str) -> Result<(), Error<T::Error>> {
                io.batch(sql) $($await)* .map_err(Error::Database)
            }

            /// Reads the connection, rejecting one inside a transaction or without a
            /// selected database.
            $($async)* fn describe_connection<T: Io<State = State>>(
                io: &mut T,
            ) -> Result<Connection, Error<T::Error>> {
                let in_transaction =
                    || Error::History("connection is already inside an application transaction".into());

                io.clean() $($await)* .map_err(Error::Database)?;

                let rows = query(
                    io,
                    sql::DESCRIBE,
                    &[],
                    &[SqlType::I64, SqlType::Text, SqlType::Text],
                ) $($await)*?;
                let [autocommit, database, sql_mode] =
                    <[Value; 3]>::try_from(single(rows, "connection description")?)
                        .map_err(|_| Error::History("connection did not describe its database".into()))?;

                if int(&autocommit) != Some(1) {
                    return Err(in_transaction());
                }

                match io.batch(sql::TRANSACTION_PROBE) $($await)* {
                    Err(error) if error.sqlstate().as_deref() == Some(sql::IN_TRANSACTION) => {
                        return Err(in_transaction());
                    }
                    probed => probed.map_err(Error::Database)?,
                }

                let Some(database) = text(database) else {
                    return Err(Error::History(
                        "migration connections require a selected database".into(),
                    ));
                };

                Ok(Connection {
                    database,
                    sql_mode: text(sql_mode).unwrap_or_default(),
                })
            }

            /// Verifies the runner still holds its named lock on this session.
            $($async)* fn verify_owner<T: Io<State = State>>(io: &mut T) -> Result<(), Error<T::Error>> {
                let lost =
                    || Error::History("migration ownership was lost or connection is uncertain".into());
                let state = io.state();
                let Some(lock) = state
                    .owner
                    .as_ref()
                    .filter(|_| !state.poisoned)
                    .map(|owner| bound(&owner.lock))
                else {
                    return Err(lost());
                };
                let held = scalar(io, sql::HOLDS_LOCK, &[lock], SqlType::I64) $($await)*?;

                if int(&held) != Some(1) {
                    io.state().poisoned = true;

                    return Err(lost());
                }

                Ok(())
            }

            /// Takes the named session lock, waiting at most the lock timeout.
            /// Named locks survive commits and end with the session.
            $($async)* fn lock<T: Io<State = State>>(io: &mut T, name: &str) -> Result<(), Error<T::Error>> {
                let seconds = io.state().lock_seconds();
                let acquired = scalar(
                    io,
                    "SELECT GET_LOCK(?, ?)",
                    &[bound(name), Value::I64(seconds)],
                    SqlType::I64,
                ) $($await)*?;

                match int(&acquired) {
                    Some(1) => Ok(()),
                    Some(0) => Err(Error::Busy),
                    _ => Err(lock_failed("GET_LOCK")),
                }
            }

            /// Restores the session after a failed acquisition; a lock or mode that
            /// cannot be restored poisons the connection, which must then be closed.
            $($async)* fn release_after_failure<T: Io<State = State>>(io: &mut T, name: &str, sql_mode: &str) {
                let restored = io.execute("SET SESSION sql_mode = ?", &[bound(sql_mode)]) $($await)*;
                let released = scalar(io, "SELECT RELEASE_LOCK(?)", &[bound(name)], SqlType::I64) $($await)*;

                if restored.is_err() || !matches!(released.map(|value| int(&value)), Ok(Some(1))) {
                    io.state().poisoned = true;
                }
            }

            /// Whether the database holds reserved tables, which must then be a valid journal.
            $($async)* fn has_journal<T: Io<State = State>>(io: &mut T) -> Result<bool, Error<T::Error>> {
                let count = scalar(io, sql::RESERVED_COUNT, &[], SqlType::I64) $($await)*?;

                if int(&count) == Some(0) {
                    return Ok(false);
                }

                validate_schema(io) $($await)*?;

                Ok(true)
            }

            $($async)* fn validate_schema<T: Io<State = State>>(io: &mut T) -> Result<(), Error<T::Error>> {
                let actual = scalar(io, sql::SIGNATURE, &[bound(sql::JOURNAL)], SqlType::Text) $($await)*?;

                if text(actual).as_deref() != Some(sql::COLUMNS) {
                    return Err(Error::History(
                        "the migration journal has an incompatible schema".into(),
                    ));
                }

                let rows = query(io, sql::INTEGRITY, &[bound(CHECKSUM_FORMAT)], &[SqlType::I64; 5]) $($await)*?;
                let counts: Vec<Option<i64>> = single(rows, "journal integrity")?.iter().map(int).collect();

                if counts != [Some(0), Some(0), Some(0), Some(1), Some(1)] {
                    return Err(Error::History(
                        "journal ownership or singleton header is invalid".into(),
                    ));
                }

                Ok(())
            }

            /// Classifies the database and creates the journal on a fresh target,
            /// then checks the stream and outstanding attempts, under the named lock.
            ///
            /// MySQL commits `CREATE TABLE` implicitly, so creation is not atomic
            /// with the header: an interrupted bootstrap leaves a journal without
            /// one, which the next run refuses as invalid instead of completing it.
            $($async)* fn bootstrap<T: Io<State = State>>(
                io: &mut T,
                stream: &str,
                scheme: VersionScheme,
            ) -> Result<(), Error<T::Error>> {
                if !has_journal(io) $($await)*? {
                    let objects = scalar(io, sql::USER_OBJECTS, &[], SqlType::I64) $($await)*?;

                    if int(&objects) != Some(0) {
                        return Err(Error::Unmanaged);
                    }

                    batch(io, sql::CREATE) $($await)*?;
                    execute(
                        io,
                        sql::INSERT_HEADER,
                        &[bound(stream), bound(scheme.as_str()), bound(CHECKSUM_FORMAT)],
                    ) $($await)*?;
                    validate_schema(io) $($await)*?;
                }

                if let Some(attempt) = check_header(io, stream, scheme) $($await)*? {
                    return Err(Error::Uncertain(attempt));
                }

                Ok(())
            }

            /// Checks the stored stream and scheme, returning the oldest unresolved attempt.
            $($async)* fn check_header<T: Io<State = State>>(
                io: &mut T,
                stream: &str,
                scheme: VersionScheme,
            ) -> Result<Option<i64>, Error<T::Error>> {
                let rows = query(
                    io,
                    sql::HEADER,
                    &[],
                    &[SqlType::Text, SqlType::Text, SqlType::I64],
                ) $($await)*?;
                let Some(header) = rows.into_iter().next() else {
                    return Err(Error::History("missing journal header".into()));
                };
                let [stored_stream, stored_scheme, unresolved] = <[Value; 3]>::try_from(header)
                    .map_err(|_| Error::History("invalid journal header".into()))?;

                if text(stored_stream).as_deref() != Some(stream)
                    || text(stored_scheme).as_deref() != Some(scheme.as_str())
                {
                    return Err(Error::History(
                        "stream or version scheme differs from managed history".into(),
                    ));
                }

                Ok(int(&unresolved))
            }

            /// Records the attempt applied, after the body's effects already committed.
            $($async)* fn checked_completion<T: Io<State = State>>(
                io: &mut T,
                attempt: i64,
            ) -> Result<(), Error<T::Error>> {
                let state = io.state();

                if state.attempt != Some(attempt) || !state.body {
                    return Err(Error::History(
                        "attempt completion did not match the running attempt".into(),
                    ));
                }

                verify_owner(io) $($await)*?;
                validate_schema(io) $($await)*?;

                let count = execute(io, sql::FINISH, &finish(attempt, status::APPLIED, None)) $($await)*?;

                if count != 1 {
                    return Err(Error::History(
                        "attempt completion did not match a running attempt".into(),
                    ));
                }

                Ok(())
            }

            /// Takes the named lock, sets the migration `sql_mode`, and validates
            /// or creates the journal. MySQL journals no transactional
            /// attempts, so there are none to reconcile.
            pub $($async)* fn acquire<T: Io<State = State>>(
                io: &mut T,
                stream: &str,
                scheme: VersionScheme,
            ) -> Result<Vec<i64>, Error<T::Error>> {
                io.state().idle()?;

                let connection = describe_connection(io) $($await)*?;
                let lock_name = connection.lock();

                lock(io, &lock_name) $($await)*?;

                let mut prepared = execute(
                    io,
                    "SET SESSION sql_mode = ?",
                    &[Value::Text(connection.migration_sql_mode())],
                ) $($await)*;

                if prepared.is_ok() {
                    prepared = bootstrap(io, stream, scheme) $($await)* .map(|()| 0);
                }

                if let Err(error) = prepared {
                    release_after_failure(io, &lock_name, &connection.sql_mode) $($await)*;

                    return Err(error);
                }

                io.state().owner = Some(Owner {
                    lock: lock_name,
                    sql_mode: connection.sql_mode,
                });

                Ok(Vec::new())
            }

            /// Reads installed history from the selected database.
            pub $($async)* fn history<T: Io<State = State>>(
                io: &mut T,
            ) -> Result<Vec<HistoryEntry>, Error<T::Error>> {
                io.clean() $($await)* .map_err(Error::Database)?;

                if !has_journal(io) $($await)*? {
                    return Ok(Vec::new());
                }

                let rows = query(io, sql::HISTORY, &[], &HISTORY_COLUMNS) $($await)*?;

                decode_history(rows)
            }

            /// Journals a running attempt.
            pub $($async)* fn start_attempt<T: Io<State = State>>(
                io: &mut T,
                entry: &CatalogEntry,
            ) -> Result<i64, Error<T::Error>> {
                verify_owner(io) $($await)*?;

                let state = io.state();

                if state.attempt.is_some() || state.body {
                    return Err(Error::History(
                        "a previous attempt remains unresolved".into(),
                    ));
                }

                let inserted = io
                    .execute(
                        sql::INSERT_ATTEMPT,
                        &[
                            Value::Text(entry.version.to_string()),
                            Value::Bytes(entry.checksum.as_bytes().to_vec()),
                            policy(entry.metadata.policy),
                        ],
                    )
                    $($await)*;

                if let Err(error) = inserted {
                    io.state().poisoned = true;

                    return Err(Error::Database(error));
                }

                let id = scalar(io, "SELECT LAST_INSERT_ID()", &[], SqlType::I64) $($await)*?;
                let Some(id) = int(&id).filter(|id| *id > 0) else {
                    io.state().poisoned = true;

                    return Err(Error::History("invalid attempt id".into()));
                };

                io.state().attempt = Some(id);

                Ok(id)
            }

            /// The body is marked begun only after ownership is verified, so a
            /// failure here records a replayable failed attempt: no statement ran.
            pub $($async)* fn begin_migration<T: Io<State = State>>(
                io: &mut T,
                policy: Policy,
            ) -> Result<(), Error<T::Error>> {
                if !super::POLICIES.contains(&policy) {
                    return Err(Error::History(format!(
                        "MySQL migration sessions cannot run {policy:?} bodies"
                    )));
                }

                verify_owner(io) $($await)*?;
                io.state().body = true;

                Ok(())
            }

            /// Records the attempt applied, poisoning the session on failure.
            /// The journal row already names the entry it runs.
            pub $($async)* fn complete<T: Io<State = State>>(
                io: &mut T,
                attempt: i64,
                _entry: &CatalogEntry,
            ) -> Result<(), Error<T::Error>> {
                let result = checked_completion(io, attempt) $($await)*;
                let state = io.state();

                if result.is_err() {
                    state.poisoned = true;
                } else {
                    state.attempt = None;
                    state.body = false;
                    state.dispatched = false;
                }

                result
            }

            /// Once a statement was dispatched its effects may persist, so the
            /// attempt stays durably unresolved rather than being recorded as failed,
            /// which would permit a replay. A body refused before dispatching any
            /// statement records an ordinary failure.
            pub $($async)* fn abort<T: Io<State = State>>(
                io: &mut T,
                attempt: i64,
            ) -> Result<(), Error<T::Error>> {
                let state = io.state();

                state.body = false;

                if std::mem::take(&mut state.dispatched) {
                    state.attempt = None;

                    return Err(Error::Uncertain(attempt));
                }

                verify_owner(io) $($await)*?;

                let count = execute(io, sql::FINISH, &finish(attempt, status::FAILED, None)) $($await)*?;
                let state = io.state();

                if count != 1 || state.attempt != Some(attempt) {
                    state.poisoned = true;

                    return Err(Error::History("cannot reconcile failed attempt".into()));
                }

                state.attempt = None;

                Ok(())
            }

            /// Restores the `sql_mode` and releases the named lock once no attempt
            /// or body is unresolved.
            pub $($async)* fn release<T: Io<State = State>>(io: &mut T) -> Result<(), Error<T::Error>> {
                verify_owner(io) $($await)*?;

                let state = io.state();

                if state.attempt.is_some() || state.body {
                    state.poisoned = true;

                    return Err(Error::History(
                        "unresolved attempt; retaining the named lock".into(),
                    ));
                }

                let Some((lock, sql_mode)) = state
                    .owner
                    .as_ref()
                    .map(|owner| (bound(&owner.lock), bound(&owner.sql_mode)))
                else {
                    return Err(Error::History(
                        "migration connection does not own the target".into(),
                    ));
                };
                let mut released = execute(io, "SET SESSION sql_mode = ?", &[sql_mode]) $($await)* .map(|_| None);

                if released.is_ok() {
                    released = scalar(io, "SELECT RELEASE_LOCK(?)", &[lock], SqlType::I64)
                        $($await)*
                        .map(|value| int(&value));
                }

                let state = io.state();

                match released {
                    Ok(Some(1)) => {
                        state.owner = None;

                        Ok(())
                    }
                    Ok(_) => {
                        state.poisoned = true;

                        Err(Error::History("named lock changed during release".into()))
                    }
                    Err(error) => {
                        state.poisoned = true;

                        Err(error)
                    }
                }
            }

            /// Sends each statement as its own request: MySQL commits DDL implicitly,
            /// so the first failure stops the script and earlier statements stay.
            pub $($async)* fn script<T: Io<State = State>>(
                io: &mut T,
                sql: &str,
            ) -> Result<(), Error<T::Error>> {
                io.state()
                    .user_state()
                    .map_err(|state| Error::Database(state.into()))?;

                let statements = crate::split_script(sql, DialectId::MySql)
                    .map_err(|e| Error::Validation(e.into()))?;

                for statement in statements {
                    io.state().dispatched = true;
                    batch(io, &sql[statement]) $($await)*?;
                }

                Ok(())
            }

            super::inspect::inspect!([$($async)*] [$($await)*]);
            super::recovery::recovery!([$($async)*] [$($await)*]);
        }
    };
}

pub(super) use steps;
