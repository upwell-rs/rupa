/// The SQLite session steps for one mode, as module `$mode` over that mode's
/// `SqliteIo`: `.await` follows each IO call in async mode.
macro_rules! steps {
    ($mode:ident, [$($async:tt)*] [$($await:tt)*]) => {
        /// The SQLite session steps in this mode.
        pub mod $mode {
            use rupa_core::{SqlType, Value};

            use super::{State, sql};
            use crate::protocol::$mode::SqliteIo;
            use crate::protocol::{
                HISTORY_COLUMNS, RUNNING_COLUMNS, absent, decode_history, finish, int, attempt_row,
                records, single, status, text,
            };
            use crate::{
                CHECKSUM_FORMAT, CatalogEntry, Direction, Error, Exclusion, HistoryEntry, Inspection, Policy,
                StateError, VersionScheme,
            };

            $($async)* fn query<T: SqliteIo<State = State>>(
                io: &mut T,
                sql: &str,
                params: &[Value],
                columns: &[SqlType],
            ) -> Result<Vec<Vec<Value>>, Error<T::Error>> {
                io.query(sql, params, columns) $($await)* .map_err(Error::Database)
            }

            /// The first column of the single row `sql` returns.
            $($async)* fn scalar<T: SqliteIo<State = State>>(
                io: &mut T,
                sql: &str,
                params: &[Value],
                column: SqlType,
            ) -> Result<Value, Error<T::Error>> {
                let rows = query(io, sql, params, &[column]) $($await)*?;

                Ok(single(rows, "control query")?.swap_remove(0))
            }

            $($async)* fn execute<T: SqliteIo<State = State>>(
                io: &mut T,
                sql: &str,
                params: &[Value],
            ) -> Result<u64, Error<T::Error>> {
                io.execute(sql, params) $($await)* .map_err(Error::Database)
            }

            $($async)* fn batch<T: SqliteIo<State = State>>(io: &mut T, sql: &str) -> Result<(), Error<T::Error>> {
                io.batch(sql) $($await)* .map_err(Error::Database)
            }

            $($async)* fn autocommit<T: SqliteIo<State = State>>(io: &mut T) -> Result<bool, Error<T::Error>> {
                io.is_autocommit() $($await)* .map_err(Error::Database)
            }

            /// `value` as text.
            fn bound(value: &str) -> Value {
                Value::Text(value.to_owned())
            }

            $($async)* fn validate_connection<T: SqliteIo<State = State>>(
                io: &mut T,
            ) -> Result<(), Error<T::Error>> {
                io.clean() $($await)* .map_err(Error::Database)?;

                if !autocommit(io) $($await)*? {
                    return Err(Error::History(
                        "connection is already inside an application transaction".into(),
                    ));
                }

                let writable = scalar(io, sql::WRITABLE_SCHEMA, &[], SqlType::I64) $($await)*?;

                if int(&writable) != Some(0) {
                    return Err(Error::History(
                        "writable_schema must be disabled on migration connections".into(),
                    ));
                }

                let attached = scalar(io, sql::ATTACHED, &[], SqlType::I64) $($await)*?;
                let temporary = scalar(io, sql::TEMPORARY, &[], SqlType::I64) $($await)*?;

                if int(&attached) != Some(0) || int(&temporary) != Some(0) {
                    return Err(Error::History(
                        "migration sessions require only main and an empty temp namespace".into(),
                    ));
                }

                Ok(())
            }

            $($async)* fn verify_owner<T: SqliteIo<State = State>>(io: &mut T) -> Result<(), Error<T::Error>> {
                let Some(owner) = io.state().owner.clone() else {
                    return Err(Error::History("migration connection does not own the target".into()));
                };
                let stored = scalar(io, sql::OWNER, &[], SqlType::Text) $($await)*?;

                if text(stored).as_deref() != Some(owner.as_str()) || io.state().poisoned {
                    return Err(Error::History(
                        "migration ownership was lost or connection is uncertain".into(),
                    ));
                }

                Ok(())
            }

            $($async)* fn has_journal<T: SqliteIo<State = State>>(io: &mut T) -> Result<bool, Error<T::Error>> {
                let names = query(io, sql::RESERVED, &[], &[SqlType::Text]) $($await)*?;

                if names.is_empty() {
                    return Ok(false);
                }

                validate_schema(io) $($await)*?;

                Ok(true)
            }

            $($async)* fn validate_schema<T: SqliteIo<State = State>>(io: &mut T) -> Result<(), Error<T::Error>> {
                let actual = query(io, sql::TABLE_SQL, &[bound(sql::JOURNAL)], &[SqlType::Text]) $($await)*?;
                let actual = actual.into_iter().next().and_then(|mut row| text(row.swap_remove(0)));

                if actual.as_deref() != Some(sql::definition()) {
                    return Err(Error::History(
                        "the migration journal has an incompatible schema".into(),
                    ));
                }

                let extra = scalar(io, sql::EXTRA, &[], SqlType::I64) $($await)*?;
                let headers = query(io, sql::HEADERS, &[bound(CHECKSUM_FORMAT)], &[SqlType::I64; 2]) $($await)*?;
                let headers: Vec<Option<i64>> = single(headers, "journal headers")?.iter().map(int).collect();

                if int(&extra) != Some(0) || headers != [Some(1), Some(1)] {
                    return Err(Error::History(
                        "journal ownership/signature or singleton header is invalid".into(),
                    ));
                }

                let maximum = scalar(io, sql::MAX_ID, &[], SqlType::I64) $($await)*?;
                let sequences = query(io, sql::SEQUENCE, &[], &[SqlType::I64]) $($await)*?;
                let sequences: Vec<Option<i64>> = sequences.iter().map(|row| int(&row[0])).collect();
                let maximum = int(&maximum);

                if !(sequences.is_empty() && maximum == Some(0) || sequences == [maximum]) {
                    return Err(Error::History(
                        "the migration journal's id sequence was changed".into(),
                    ));
                }

                Ok(())
            }

            /// Checks the stored stream and scheme, returning the owner claim.
            $($async)* fn check_header<T: SqliteIo<State = State>>(
                io: &mut T,
                stream: &str,
                scheme: VersionScheme,
            ) -> Result<Option<String>, Error<T::Error>> {
                let rows = query(io, sql::HEADER, &[], &[SqlType::Text; 3]) $($await)*?;
                let [stored_stream, stored_scheme, owner] =
                    <[Value; 3]>::try_from(single(rows, "journal header")?)
                        .map_err(|_| Error::History("invalid journal header".into()))?;

                if text(stored_stream).as_deref() != Some(stream)
                    || text(stored_scheme).as_deref() != Some(scheme.as_str())
                {
                    return Err(Error::History(
                        "stream or version scheme differs from managed history".into(),
                    ));
                }

                Ok(text(owner))
            }

            /// Records the running transactional attempts as rolled back. Once
            /// no runner owns the target, their transactions have ended, and
            /// completion commits with the body, so none of them committed.
            $($async)* fn reconcile<T: SqliteIo<State = State>>(io: &mut T) -> Result<Vec<i64>, Error<T::Error>> {
                let rows = query(io, sql::RECONCILABLE, &[], &[SqlType::I64]) $($await)*?;

                execute(io, sql::RECONCILE, &[]) $($await)*?;

                Ok(rows.iter().filter_map(|row| int(&row[0])).collect())
            }

            /// Classifies `main`, creates the journal on a fresh target, checks
            /// the stream and outstanding attempts, reconciles interrupted
            /// transactional ones, and claims the target durably.
            $($async)* fn claim<T: SqliteIo<State = State>>(
                io: &mut T,
                stream: &str,
                scheme: VersionScheme,
            ) -> Result<(String, Vec<i64>), Error<T::Error>> {
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
                }

                if check_header(io, stream, scheme) $($await)*?.is_some() {
                    return Err(Error::Busy);
                }

                let unresolved = query(io, sql::UNRESOLVED, &[], &[SqlType::I64]) $($await)*?;

                if let Some(attempt) = unresolved.first().and_then(|row| int(&row[0])) {
                    return Err(Error::Uncertain(attempt));
                }

                let reconciled = reconcile(io) $($await)*?;
                let token = scalar(io, sql::TOKEN, &[], SqlType::Text) $($await)*?;
                let Some(token) = text(token) else {
                    return Err(Error::History("could not create an owner token".into()));
                };

                execute(io, sql::CLAIM, &[bound(&token)]) $($await)*?;

                Ok((token, reconciled))
            }

            /// Admits user statements: owned, unpoisoned, inside the attempt's
            /// transaction. Installs the authorizer; [`user_end`] removes it.
            pub $($async)* fn user_begin<T: SqliteIo<State = State>>(io: &mut T) -> Result<(), T::Error> {
                io.state().user_state()?;

                if io.is_autocommit() $($await)*? {
                    return Err(StateError::NoTransaction.into());
                }

                io.authorizer(true) $($await)*
            }

            /// Removes the authorizer [`user_begin`] installed.
            pub $($async)* fn user_end<T: SqliteIo<State = State>>(io: &mut T) -> Result<(), T::Error> {
                io.authorizer(false) $($await)*
            }

            /// Sets the connection's `busy_timeout`; a timeout that cannot be
            /// set back poisons the connection, which must then be closed.
            $($async)* fn set_busy_timeout<T: SqliteIo<State = State>>(
                io: &mut T,
                millis: i64,
            ) -> Result<(), Error<T::Error>> {
                let set = io.batch(&format!("PRAGMA busy_timeout = {millis}")) $($await)*;

                set.map_err(|error| {
                    io.state().poisoned = true;

                    Error::Database(error)
                })
            }

            /// Opens a `BEGIN IMMEDIATE` writer transaction on a clean session,
            /// waiting at most the lock timeout, and returns the connection's
            /// own `busy_timeout`, which is restored here if it fails.
            $($async)* fn begin_writer<T: SqliteIo<State = State>>(io: &mut T) -> Result<i64, Error<T::Error>> {
                let state = io.state();
                let busy = state.owner.is_some() || state.poisoned;
                let millis = state.lock_timeout.as_millis().min(i32::MAX as u128) as i64;

                if busy || !autocommit(io) $($await)*? {
                    return Err(Error::History(
                        "session already owns a run or is not clean".into(),
                    ));
                }

                validate_connection(io) $($await)*?;

                let original = scalar(io, "PRAGMA busy_timeout", &[], SqlType::I64) $($await)*?;
                let original = int(&original).unwrap_or(0);

                batch(io, &format!("PRAGMA busy_timeout = {millis}")) $($await)*?;

                if let Err(error) = io.batch("BEGIN IMMEDIATE") $($await)* {
                    set_busy_timeout(io, original) $($await)*?;

                    return Err(Error::Database(error));
                }

                Ok(original)
            }

            /// Claims the target under the writer lock, waiting at most the lock
            /// timeout. The connection's own `busy_timeout` is restored at
            /// release, or here when the claim fails.
            pub $($async)* fn acquire<T: SqliteIo<State = State>>(
                io: &mut T,
                stream: &str,
                scheme: VersionScheme,
            ) -> Result<Vec<i64>, Error<T::Error>> {
                let original = begin_writer(io) $($await)*?;

                match claim(io, stream, scheme) $($await)* {
                    Ok((token, reconciled)) => {
                        let state = io.state();

                        state.owner = Some(token);
                        state.busy_timeout = Some(original);

                        if let Err(error) = io.batch("COMMIT") $($await)* {
                            io.state().poisoned = true;

                            return Err(Error::Database(error));
                        }

                        Ok(reconciled)
                    }
                    Err(error) => {
                        if io.batch("ROLLBACK") $($await)* .is_err() {
                            io.state().poisoned = true;
                        }

                        set_busy_timeout(io, original) $($await)*?;

                        Err(error)
                    }
                }
            }

            /// Reads installed history from `main`.
            pub $($async)* fn history<T: SqliteIo<State = State>>(
                io: &mut T,
            ) -> Result<Vec<HistoryEntry>, Error<T::Error>> {
                if !has_journal(io) $($await)*? {
                    return Ok(Vec::new());
                }

                let rows = query(io, sql::HISTORY, &[], &HISTORY_COLUMNS) $($await)*?;

                decode_history(rows)
            }

            /// Journals a running attempt of `entry` in `direction`.
            pub $($async)* fn start_attempt<T: SqliteIo<State = State>>(
                io: &mut T,
                entry: &CatalogEntry,
                direction: Direction,
            ) -> Result<i64, Error<T::Error>> {
                verify_owner(io) $($await)*?;

                if io.state().attempt.is_some() || !autocommit(io) $($await)*? {
                    return Err(Error::History(
                        "a previous attempt remains unresolved".into(),
                    ));
                }

                let inserted = io
                    .execute(
                        sql::INSERT_ATTEMPT,
                        &attempt_row(entry, direction),
                    )
                    $($await)*;

                if let Err(error) = inserted {
                    io.state().poisoned = true;

                    return Err(Error::Database(error));
                }

                let id = scalar(io, "SELECT last_insert_rowid()", &[], SqlType::I64) $($await)*?;
                let Some(id) = int(&id) else {
                    io.state().poisoned = true;

                    return Err(Error::History("invalid attempt id".into()));
                };

                io.state().attempt = Some(id);

                Ok(id)
            }

            /// Opens the attempt's immediate transaction.
            pub $($async)* fn begin_migration<T: SqliteIo<State = State>>(
                io: &mut T,
                policy: Policy,
            ) -> Result<(), Error<T::Error>> {
                if !super::POLICIES.contains(&policy) {
                    return Err(Error::History(format!(
                        "SQLite migration sessions cannot run {policy:?} bodies"
                    )));
                }

                batch(io, "BEGIN IMMEDIATE") $($await)*?;

                verify_owner(io) $($await)*
            }

            $($async)* fn record_completion<T: SqliteIo<State = State>>(
                io: &mut T,
                attempt: i64,
            ) -> Result<(), Error<T::Error>> {
                let count = execute(io, sql::FINISH, &finish(attempt, status::APPLIED, None)) $($await)*?;

                if count != 1 {
                    return Err(Error::History(
                        "attempt completion did not match a running attempt".into(),
                    ));
                }

                batch(io, "COMMIT") $($await)*
            }

            /// Commits the attempt's completion with the artifact's effects. The
            /// journal row already names the entry it runs.
            pub $($async)* fn complete<T: SqliteIo<State = State>>(
                io: &mut T,
                attempt: i64,
                _entry: &CatalogEntry,
            ) -> Result<(), Error<T::Error>> {
                verify_owner(io) $($await)*?;
                validate_schema(io) $($await)*?;

                if io.state().attempt != Some(attempt) || autocommit(io) $($await)*? {
                    return Err(Error::History(
                        "artifact transaction ended outside runner control".into(),
                    ));
                }

                let result = record_completion(io, attempt) $($await)*;
                let state = io.state();

                if result.is_err() {
                    state.poisoned = true;
                } else {
                    state.attempt = None;
                }

                result
            }

            /// Rolls back the body and records its failure.
            pub $($async)* fn abort<T: SqliteIo<State = State>>(
                io: &mut T,
                attempt: i64,
            ) -> Result<(), Error<T::Error>> {
                if !autocommit(io) $($await)*?
                    && let Err(error) = io.batch("ROLLBACK") $($await)*
                {
                    io.state().poisoned = true;

                    return Err(Error::Database(error));
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

            /// Releases the durable claim once no attempt or transaction is unresolved.
            pub $($async)* fn release<T: SqliteIo<State = State>>(io: &mut T) -> Result<(), Error<T::Error>> {
                verify_owner(io) $($await)*?;

                if io.state().attempt.is_some() || !autocommit(io) $($await)*? {
                    io.state().poisoned = true;

                    return Err(Error::History(
                        "unresolved attempt/transaction; retaining durable owner claim".into(),
                    ));
                }

                let owner = io.state().owner.take().unwrap_or_default();
                let released = io.execute(sql::UNCLAIM, &[bound(&owner)]) $($await)*;

                if matches!(released, Ok(1)) {
                    let original = io.state().busy_timeout.take().unwrap_or(0);

                    return set_busy_timeout(io, original) $($await)*;
                }

                let state = io.state();

                state.owner = Some(owner);
                state.poisoned = true;

                match released {
                    Err(error) => Err(Error::Database(error)),
                    _ => Err(Error::History("owner claim changed during release".into())),
                }
            }

            /// Runs an authorized user script in the attempt's transaction.
            pub $($async)* fn script<T: SqliteIo<State = State>>(
                io: &mut T,
                sql: &str,
            ) -> Result<(), Error<T::Error>> {
                user_begin(io) $($await)* .map_err(Error::Database)?;

                let result = io.batch(sql) $($await)*;
                let ended = user_end(io) $($await)*;

                result.and(ended).map_err(Error::Database)
            }

            super::inspect::inspect!([$($async)*] [$($await)*]);
            super::recovery::recovery!([$($async)*] [$($await)*]);
        }
    };
}

pub(super) use steps;
