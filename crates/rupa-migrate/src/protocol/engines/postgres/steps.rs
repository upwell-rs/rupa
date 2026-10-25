/// The PostgreSQL session steps for one mode, as module `$mode` over that
/// mode's `Io`: `.await` follows each IO call in async mode.
macro_rules! steps {
    ($mode:ident, [$($async:tt)*] [$($await:tt)*]) => {
        /// The PostgreSQL session steps in this mode.
        pub mod $mode {
            use rupa_core::{DialectId, SqlType, Value};

            use super::{Body, Connection, LOCK_NOT_AVAILABLE, Owner, State, lock_parts, quote, sql};
            use crate::protocol::$mode::Io;
            use crate::protocol::{HISTORY_COLUMNS, SqlState as _, decode_history, int, single, text};
            use crate::{CHECKSUM_FORMAT, CatalogEntry, Error, HistoryEntry, Policy, VersionScheme};

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

            $($async)* fn describe_connection<T: Io<State = State>>(
                io: &mut T,
            ) -> Result<Connection, Error<T::Error>> {
                io.clean() $($await)* .map_err(Error::Database)?;

                let rows = query(
                    io,
                    sql::DESCRIBE,
                    &[],
                    &[SqlType::Text, SqlType::Text, SqlType::Text, SqlType::Text],
                ) $($await)*?;
                let row = single(rows, "connection description")?;
                let again = scalar(io, sql::TRANSACTION_START, &[], SqlType::Text) $($await)*?;

                Connection::describe(row, text(again))
            }

            /// Verifies the runner still holds its advisory lock on this session.
            $($async)* fn verify_owner<T: Io<State = State>>(io: &mut T) -> Result<(), Error<T::Error>> {
                let lost = || {
                    Error::History("migration ownership was lost or connection is uncertain".into())
                };
                let state = io.state();
                let key = match &state.owner {
                    Some(owner) if !state.poisoned => owner.key,
                    _ => return Err(lost()),
                };
                let held = scalar(io, sql::HOLDS_LOCK, &lock_parts(key), SqlType::Bool) $($await)*?;

                if held != Value::Bool(true) {
                    io.state().poisoned = true;

                    return Err(lost());
                }

                Ok(())
            }

            /// Takes the session advisory lock, waiting at most the lock timeout.
            /// Session locks survive the transaction that bounds the wait.
            $($async)* fn lock<T: Io<State = State>>(io: &mut T, key: i64) -> Result<(), Error<T::Error>> {
                let state = io.state();
                let millis = state.lock_millis();
                let wait = !state.lock_timeout.is_zero();

                if !wait {
                    let acquired = scalar(
                        io,
                        "SELECT pg_try_advisory_lock($1)",
                        &[Value::I64(key)],
                        SqlType::Bool,
                    ) $($await)*?;

                    return if acquired == Value::Bool(true) { Ok(()) } else { Err(Error::Busy) };
                }

                let bounded = io
                    .batch(&format!("BEGIN; SET LOCAL lock_timeout = '{millis}ms'"))
                    $($await)*;

                if let Err(error) = bounded {
                    if io.batch("ROLLBACK") $($await)* .is_err() {
                        io.state().poisoned = true;
                    }

                    return Err(Error::Database(error));
                }

                let locked = io.execute("SELECT pg_advisory_lock($1)", &[Value::I64(key)]) $($await)*;

                if let Err(error) = locked {
                    let busy = error.sqlstate().as_deref() == Some(LOCK_NOT_AVAILABLE);

                    if io.batch("ROLLBACK") $($await)* .is_err() {
                        io.state().poisoned = true;
                    }

                    return Err(if busy { Error::Busy } else { Error::Database(error) });
                }

                let committed = io.batch("COMMIT") $($await)*;

                committed.map_err(|error| {
                    io.state().poisoned = true;

                    Error::Database(error)
                })
            }

            /// Releases the lock after a failed acquisition; a lock that cannot be
            /// released poisons the connection, which must then be closed.
            $($async)* fn unlock_after_failure<T: Io<State = State>>(io: &mut T, key: i64) {
                if io.batch("ROLLBACK") $($await)* .is_err() {
                    io.state().poisoned = true;
                }

                let released = scalar(
                    io,
                    "SELECT pg_advisory_unlock($1)",
                    &[Value::I64(key)],
                    SqlType::Bool,
                ) $($await)*;

                if !matches!(released, Ok(Value::Bool(true))) {
                    io.state().poisoned = true;
                }
            }

            $($async)* fn current_transaction<T: Io<State = State>>(
                io: &mut T,
            ) -> Result<i64, Error<T::Error>> {
                let id = scalar(io, "SELECT txid_current()", &[], SqlType::I64) $($await)*?;

                int(&id).ok_or_else(|| Error::History("txid_current() returned no id".into()))
            }

            /// Whether the namespace holds reserved objects, which must then be valid metadata.
            $($async)* fn has_metadata<T: Io<State = State>>(
                io: &mut T,
                namespace: &str,
            ) -> Result<bool, Error<T::Error>> {
                let count = scalar(
                    io,
                    sql::RESERVED_COUNT,
                    &[Value::Text(namespace.to_owned())],
                    SqlType::I64,
                ) $($await)*?;

                if int(&count) == Some(0) {
                    return Ok(false);
                }

                validate_schema(io, namespace) $($await)*?;

                Ok(true)
            }

            $($async)* fn validate_schema<T: Io<State = State>>(
                io: &mut T,
                namespace: &str,
            ) -> Result<(), Error<T::Error>> {
                for (table, _, expected) in sql::TABLES {
                    let actual = scalar(
                        io,
                        sql::SIGNATURE,
                        &[Value::Text(namespace.to_owned()), Value::Text(table.to_owned())],
                        SqlType::Text,
                    ) $($await)*?;

                    if text(actual).as_deref() != Some(expected) {
                        return Err(Error::History(format!(
                            "reserved metadata table {table} has an incompatible schema"
                        )));
                    }
                }

                let rows = query(
                    io,
                    &sql::integrity(&quote(namespace)),
                    &[Value::Text(namespace.to_owned()), Value::Text(CHECKSUM_FORMAT.to_owned())],
                    &[SqlType::I64, SqlType::I64, SqlType::I64],
                ) $($await)*?;
                let row = single(rows, "metadata integrity")?;
                let counts: Vec<Option<i64>> = row.iter().map(int).collect();

                if counts != [Some(0), Some(0), Some(1)] {
                    return Err(Error::History(
                        "metadata ownership/signature or singleton header is invalid".into(),
                    ));
                }

                Ok(())
            }

            /// Classifies the namespace and creates metadata on a fresh target,
            /// then checks the stream and outstanding attempts, in one
            /// transaction under the advisory lock.
            $($async)* fn bootstrap<T: Io<State = State>>(
                io: &mut T,
                namespace: &str,
                stream: &str,
                scheme: VersionScheme,
            ) -> Result<(), Error<T::Error>> {
                let ns = quote(namespace);

                batch(io, "BEGIN") $($await)*?;

                if !has_metadata(io, namespace) $($await)*? {
                    let objects = scalar(
                        io,
                        sql::USER_OBJECTS,
                        &[Value::Text(namespace.to_owned())],
                        SqlType::I64,
                    ) $($await)*?;

                    if int(&objects) != Some(0) {
                        return Err(Error::Unmanaged);
                    }

                    for (_, ddl, _) in sql::TABLES {
                        batch(io, &ddl.replace("{ns}", &ns)) $($await)*?;
                    }

                    execute(
                        io,
                        &sql::insert_header(&ns),
                        &[
                            Value::Text(stream.to_owned()),
                            Value::Text(scheme.as_str().to_owned()),
                            Value::Text(CHECKSUM_FORMAT.to_owned()),
                        ],
                    ) $($await)*?;
                }

                let rows = query(
                    io,
                    &sql::header(&ns),
                    &[],
                    &[SqlType::Text, SqlType::Text, SqlType::I64],
                ) $($await)*?;
                let [stored_stream, stored_scheme, unresolved] =
                    <[Value; 3]>::try_from(single(rows, "metadata header")?)
                        .map_err(|_| Error::History("invalid metadata header".into()))?;

                if text(stored_stream).as_deref() != Some(stream)
                    || text(stored_scheme).as_deref() != Some(scheme.as_str())
                {
                    return Err(Error::History(
                        "stream or version scheme differs from managed history".into(),
                    ));
                }

                if let Some(attempt) = int(&unresolved) {
                    return Err(Error::Uncertain(attempt));
                }

                batch(io, "COMMIT") $($await)*
            }

            /// Commits history and attempt completion with the artifact's
            /// effects. A transactional body must have left the artifact
            /// transaction it was given: the transaction id proves no statement
            /// committed or replaced it. A nontransactional body's effects are
            /// already committed, so completion opens its own transaction.
            $($async)* fn checked_completion<T: Io<State = State>>(
                io: &mut T,
                attempt: i64,
                entry: &CatalogEntry,
            ) -> Result<(), Error<T::Error>> {
                let state = io.state();
                let body = state.body;

                if state.attempt != Some(attempt) {
                    return Err(Error::History(
                        "attempt completion did not match the running attempt".into(),
                    ));
                }

                match body {
                    Some(Body::Transaction(id)) if id == current_transaction(io) $($await)*? => {}
                    Some(Body::Autocommit { .. }) => batch(io, "BEGIN") $($await)*?,
                    _ => {
                        return Err(Error::History(
                            "artifact transaction ended outside runner control".into(),
                        ));
                    }
                }

                verify_owner(io) $($await)*?;

                let state = io.state();
                let namespace = state.owned_namespace()?;
                let history = state.reserved(sql::HISTORY)?;
                let attempts = state.reserved(sql::ATTEMPTS)?;

                validate_schema(io, &namespace) $($await)*?;
                execute(
                    io,
                    &sql::insert_history(&history),
                    &[
                        Value::Text(entry.version.to_string()),
                        Value::Bytes(entry.checksum.as_bytes().to_vec()),
                    ],
                ) $($await)*?;

                let count = execute(
                    io,
                    &sql::finish_attempt(&attempts, "applied"),
                    &[Value::I64(attempt)],
                ) $($await)*?;

                if count != 1 {
                    return Err(Error::History(
                        "attempt completion did not match a running attempt".into(),
                    ));
                }

                batch(io, "COMMIT") $($await)*
            }

            /// Sends each statement of a script as its own request: several
            /// statements in one simple query share an implicit transaction,
            /// which statements such as `CREATE INDEX CONCURRENTLY` refuse. The
            /// first failure stops the script; earlier statements stay committed.
            $($async)* fn autocommit_script<T: Io<State = State>>(
                io: &mut T,
                sql: &str,
            ) -> Result<(), Error<T::Error>> {
                let statements = crate::split_script(sql, DialectId::Postgres)
                    .map_err(|e| Error::Validation(e.into()))?;

                for statement in statements {
                    io.state().dispatch();
                    batch(io, &sql[statement]) $($await)*?;
                }

                Ok(())
            }

            /// Takes the advisory lock and validates or creates the metadata.
            pub $($async)* fn acquire<T: Io<State = State>>(
                io: &mut T,
                stream: &str,
                scheme: VersionScheme,
            ) -> Result<(), Error<T::Error>> {
                let state = io.state();

                if state.owner.is_some() || state.poisoned {
                    return Err(Error::History(
                        "session already owns a run or is not clean".into(),
                    ));
                }

                let connection = describe_connection(io) $($await)*?;
                let key = connection.key();

                lock(io, key) $($await)*?;

                if let Err(error) = bootstrap(io, &connection.namespace, stream, scheme) $($await)* {
                    unlock_after_failure(io, key) $($await)*;

                    return Err(error);
                }

                io.state().owner = Some(Owner {
                    key,
                    namespace: connection.namespace,
                });

                Ok(())
            }

            /// Reads installed history from the owned or current namespace.
            pub $($async)* fn history<T: Io<State = State>>(
                io: &mut T,
            ) -> Result<Vec<HistoryEntry>, Error<T::Error>> {
                let owned = io.state().owner.as_ref().map(|owner| owner.namespace.clone());
                let namespace = match owned {
                    Some(namespace) => namespace,
                    None => describe_connection(io) $($await)*?.namespace,
                };

                if !has_metadata(io, &namespace) $($await)*? {
                    return Ok(Vec::new());
                }

                let rows = query(io, &sql::history(&quote(&namespace)), &[], &HISTORY_COLUMNS) $($await)*?;

                decode_history(rows)
            }

            /// Journals a running attempt.
            pub $($async)* fn start_attempt<T: Io<State = State>>(
                io: &mut T,
                entry: &CatalogEntry,
            ) -> Result<i64, Error<T::Error>> {
                verify_owner(io) $($await)*?;

                let state = io.state();

                if state.attempt.is_some() || state.body.is_some() {
                    return Err(Error::History(
                        "a previous attempt remains unresolved".into(),
                    ));
                }

                let attempts = state.reserved(sql::ATTEMPTS)?;
                let inserted = io
                    .query(
                        &sql::insert_attempt(&attempts),
                        &[
                            Value::Text(entry.version.to_string()),
                            Value::Bytes(entry.checksum.as_bytes().to_vec()),
                        ],
                        &[SqlType::I64],
                    )
                    $($await)*;
                let id = match inserted.map(|rows| rows.first().and_then(|row| int(&row[0]))) {
                    Ok(Some(id)) => id,
                    Ok(None) => {
                        io.state().poisoned = true;

                        return Err(Error::History("attempt insert returned no id".into()));
                    }
                    Err(error) => {
                        io.state().poisoned = true;

                        return Err(Error::Database(error));
                    }
                };

                io.state().attempt = Some(id);

                Ok(id)
            }

            /// An autocommit body is marked begun only after ownership is
            /// verified, so a failure here records a replayable failed attempt
            /// instead of an uncertain one: no statement has run yet.
            pub $($async)* fn begin_migration<T: Io<State = State>>(
                io: &mut T,
                policy: Policy,
            ) -> Result<(), Error<T::Error>> {
                io.forget_statements() $($await)* .map_err(Error::Database)?;

                if policy == Policy::NonTransactional {
                    verify_owner(io) $($await)*?;
                    io.state().body = Some(Body::Autocommit { dispatched: false });

                    return Ok(());
                }

                batch(io, "BEGIN") $($await)*?;

                let transaction = current_transaction(io) $($await)*?;

                io.state().body = Some(Body::Transaction(transaction));

                verify_owner(io) $($await)*
            }

            /// Records history and completion, poisoning the session on failure.
            pub $($async)* fn complete<T: Io<State = State>>(
                io: &mut T,
                attempt: i64,
                entry: &CatalogEntry,
            ) -> Result<(), Error<T::Error>> {
                let result = checked_completion(io, attempt, entry) $($await)*;
                let state = io.state();

                if result.is_err() {
                    state.poisoned = true;
                } else {
                    state.attempt = None;
                    state.body = None;
                }

                result
            }

            /// Once a nontransactional body dispatched a statement its effects
            /// may persist, so its attempt stays durably unresolved rather than
            /// being recorded as failed, which would permit a replay. A body
            /// refused before dispatching anything records an ordinary failure.
            pub $($async)* fn abort<T: Io<State = State>>(
                io: &mut T,
                attempt: i64,
            ) -> Result<(), Error<T::Error>> {
                match io.state().body.take() {
                    Some(Body::Autocommit { dispatched: true }) => {
                        io.state().attempt = None;

                        return Err(Error::Uncertain(attempt));
                    }
                    Some(Body::Autocommit { dispatched: false }) | None => {}
                    Some(Body::Transaction(_)) => {
                        if let Err(error) = io.batch("ROLLBACK") $($await)* {
                            io.state().poisoned = true;

                            return Err(Error::Database(error));
                        }
                    }
                }

                verify_owner(io) $($await)*?;

                let attempts = io.state().reserved(sql::ATTEMPTS)?;
                let count = execute(
                    io,
                    &sql::finish_attempt(&attempts, "failed"),
                    &[Value::I64(attempt)],
                ) $($await)*?;
                let state = io.state();

                if count != 1 || state.attempt != Some(attempt) {
                    state.poisoned = true;

                    return Err(Error::History("cannot reconcile failed attempt".into()));
                }

                state.attempt = None;

                Ok(())
            }

            /// Releases the advisory lock once no attempt or body is unresolved.
            pub $($async)* fn release<T: Io<State = State>>(io: &mut T) -> Result<(), Error<T::Error>> {
                verify_owner(io) $($await)*?;

                let state = io.state();

                if state.attempt.is_some() || state.body.is_some() {
                    state.poisoned = true;

                    return Err(Error::History(
                        "unresolved attempt/transaction; retaining the advisory lock".into(),
                    ));
                }

                let Some(key) = state.owner.as_ref().map(|owner| owner.key) else {
                    return Err(Error::History(
                        "migration connection does not own the target".into(),
                    ));
                };
                let unlocked = scalar(
                    io,
                    "SELECT pg_advisory_unlock($1)",
                    &[Value::I64(key)],
                    SqlType::Bool,
                ) $($await)*;
                let state = io.state();

                match unlocked {
                    Ok(Value::Bool(true)) => {
                        state.owner = None;

                        Ok(())
                    }
                    Ok(_) => {
                        state.poisoned = true;

                        Err(Error::History("advisory lock changed during release".into()))
                    }
                    Err(error) => {
                        state.poisoned = true;

                        Err(error)
                    }
                }
            }

            /// Runs an authorized user script in the body's transaction, or
            /// statement by statement in autocommit.
            pub $($async)* fn script<T: Io<State = State>>(
                io: &mut T,
                sql: &str,
            ) -> Result<(), Error<T::Error>> {
                let state = io.state();

                state
                    .user_state()
                    .map_err(|state| Error::Database(state.into()))?;

                let autocommit = matches!(state.body, Some(Body::Autocommit { .. }));
                let result = if autocommit {
                    autocommit_script(io, sql) $($await)*
                } else {
                    batch(io, sql) $($await)*
                };
                let forgotten = io.forget_statements() $($await)* .map_err(Error::Database);

                result.and(forgotten)
            }
        }
    };
}

pub(super) use steps;
