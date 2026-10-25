/// Ownership, locking, metadata and completion steps of the migration
/// session, for one mode: `.await` follows each client call in async mode.
#[doc(hidden)]
#[macro_export]
macro_rules! __pg_migration_control {
    ([$($async:tt)*] [$($await:tt)*] $tx:path) => {
        impl PgExecutor {
            /// Bounds how long a runner waits for another runner's advisory lock.
            /// Zero tries once without waiting; waits beyond PostgreSQL's
            /// `lock_timeout` maximum (about 24.8 days) are clamped to it.
            pub fn with_migration_lock_timeout(mut self, timeout: ::std::time::Duration) -> Self {
                self.migration.lock_timeout = timeout;

                self
            }

            $($async)* fn describe_connection(&mut self) -> Result<Connection, Error<PgError>> {
                <Self as $tx>::clean(self) $($await)* .map_err(Error::Database)?;

                let messages = self.client.simple_query(sql::DESCRIBE) $($await)* .map_err(db)?;

                Connection::describe(messages)
            }

            /// Verifies the runner still holds its advisory lock on this session.
            $($async)* fn verify_owner(&mut self) -> Result<(), Error<PgError>> {
                let lost = || {
                    Error::History("migration ownership was lost or connection is uncertain".into())
                };
                let key = match &self.migration.owner {
                    Some(owner) if !self.migration.poisoned => owner.key,
                    _ => return Err(lost()),
                };
                let (high, low) = lock_parts(key);
                let held: bool = self
                    .client
                    .query_one(sql::HOLDS_LOCK, &[&high, &low])
                    $($await)*
                    .map_err(db)?
                    .get(0);

                if !held {
                    self.migration.poisoned = true;

                    return Err(lost());
                }

                Ok(())
            }

            /// Takes the session advisory lock, waiting at most the lock timeout.
            /// Session locks survive the transaction that bounds the wait.
            $($async)* fn lock(&mut self, key: i64) -> Result<(), Error<PgError>> {
                let millis = self.migration.lock_millis();

                if self.migration.lock_timeout.is_zero() {
                    let acquired: bool = self
                        .client
                        .query_one("SELECT pg_try_advisory_lock($1)", &[&key])
                        $($await)*
                        .map_err(db)?
                        .get(0);

                    return if acquired { Ok(()) } else { Err(Error::Busy) };
                }

                let bounded = self
                    .client
                    .batch_execute(&format!("BEGIN; SET LOCAL lock_timeout = '{millis}ms'"))
                    $($await)*;

                if let Err(error) = bounded {
                    if self.client.batch_execute("ROLLBACK") $($await)* .is_err() {
                        self.migration.poisoned = true;
                    }

                    return Err(db(error));
                }

                let locked = self.client.execute("SELECT pg_advisory_lock($1)", &[&key]) $($await)*;

                if let Err(error) = locked {
                    let busy = error.code() == Some(&SqlState::LOCK_NOT_AVAILABLE);

                    if self.client.batch_execute("ROLLBACK") $($await)* .is_err() {
                        self.migration.poisoned = true;
                    }

                    return Err(if busy { Error::Busy } else { db(error) });
                }

                let committed = self.client.batch_execute("COMMIT") $($await)*;

                committed.map_err(|error| {
                    self.migration.poisoned = true;

                    db(error)
                })
            }

            /// Releases the lock after a failed acquisition; a lock that cannot be
            /// released poisons the connection, which must then be closed.
            $($async)* fn unlock_after_failure(&mut self, key: i64) {
                if self.client.batch_execute("ROLLBACK") $($await)* .is_err() {
                    self.migration.poisoned = true;
                }

                let released = self
                    .client
                    .query_one("SELECT pg_advisory_unlock($1)", &[&key])
                    $($await)*
                    .map(|row| row.get::<_, bool>(0));

                if !matches!(released, Ok(true)) {
                    self.migration.poisoned = true;
                }
            }

            $($async)* fn current_transaction(&mut self) -> Result<i64, Error<PgError>> {
                Ok(self
                    .client
                    .query_one("SELECT txid_current()", &[])
                    $($await)*
                    .map_err(db)?
                    .get(0))
            }

            /// Whether the namespace holds reserved objects, which must then be valid metadata.
            $($async)* fn has_metadata(&mut self, namespace: &str) -> Result<bool, Error<PgError>> {
                let count: i64 = self
                    .client
                    .query_one(sql::RESERVED_COUNT, &[&namespace])
                    $($await)*
                    .map_err(db)?
                    .get(0);

                if count == 0 {
                    return Ok(false);
                }

                self.validate_schema(namespace) $($await)*?;

                Ok(true)
            }

            $($async)* fn validate_schema(&mut self, namespace: &str) -> Result<(), Error<PgError>> {
                let relations = sql::RELATIONS.as_slice();
                let tables = &sql::RELATIONS[..3];

                for (table, _, expected) in sql::TABLES {
                    let actual: String = self
                        .client
                        .query_one(sql::SIGNATURE, &[&namespace, &table])
                        $($await)*
                        .map_err(db)?
                        .get(0);

                    if actual != expected {
                        return Err(Error::History(format!(
                            "reserved metadata table {table} has an incompatible schema"
                        )));
                    }
                }

                let row = self
                    .client
                    .query_one(
                        &sql::integrity(&quote(namespace)),
                        &[&namespace, &relations, &tables, &CHECKSUM_FORMAT],
                    )
                    $($await)*
                    .map_err(db)?;
                let (extra, triggers, header): (i64, i64, i64) = (row.get(0), row.get(1), row.get(2));

                if extra != 0 || triggers != 0 || header != 1 {
                    return Err(Error::History(
                        "metadata ownership/signature or singleton header is invalid".into(),
                    ));
                }

                Ok(())
            }

            /// Classifies the namespace and creates metadata on a fresh target,
            /// then checks the stream and outstanding attempts, in one
            /// transaction under the advisory lock.
            $($async)* fn bootstrap(
                &mut self,
                namespace: &str,
                stream: &str,
                scheme: VersionScheme,
            ) -> Result<(), Error<PgError>> {
                let ns = quote(namespace);

                self.client.batch_execute("BEGIN") $($await)* .map_err(db)?;

                if !self.has_metadata(namespace) $($await)*? {
                    let objects: i64 = self
                        .client
                        .query_one(sql::USER_OBJECTS, &[&namespace])
                        $($await)*
                        .map_err(db)?
                        .get(0);

                    if objects != 0 {
                        return Err(Error::Unmanaged);
                    }

                    for (_, ddl, _) in sql::TABLES {
                        self.client
                            .batch_execute(&ddl.replace("{ns}", &ns))
                            $($await)*
                            .map_err(db)?;
                    }

                    self.client
                        .execute(
                            &sql::insert_header(&ns),
                            &[&stream, &scheme.as_str(), &CHECKSUM_FORMAT],
                        )
                        $($await)*
                        .map_err(db)?;
                }

                let row = self.client.query_one(&sql::header(&ns), &[]) $($await)* .map_err(db)?;
                let (stored_stream, stored_scheme, unresolved): (String, String, Option<i64>) =
                    (row.get(0), row.get(1), row.get(2));

                if stored_stream != stream || stored_scheme != scheme.as_str() {
                    return Err(Error::History(
                        "stream or version scheme differs from managed history".into(),
                    ));
                }

                if let Some(attempt) = unresolved {
                    return Err(Error::Uncertain(attempt));
                }

                self.client.batch_execute("COMMIT") $($await)* .map_err(db)
            }

            /// Commits history and attempt completion with the artifact's
            /// effects. A transactional body must have left the artifact
            /// transaction it was given: the transaction id proves no statement
            /// committed or replaced it. A nontransactional body's effects are
            /// already committed, so completion opens its own transaction.
            $($async)* fn checked_completion(
                &mut self,
                attempt: i64,
                entry: &CatalogEntry,
            ) -> Result<(), Error<PgError>> {
                let body = self.migration.body;

                if self.migration.attempt != Some(attempt) {
                    return Err(Error::History(
                        "attempt completion did not match the running attempt".into(),
                    ));
                }

                match body {
                    Some(Body::Transaction(id)) if id == self.current_transaction() $($await)*? => {}
                    Some(Body::Autocommit { .. }) => {
                        self.client.batch_execute("BEGIN") $($await)* .map_err(db)?
                    }
                    _ => {
                        return Err(Error::History(
                            "artifact transaction ended outside runner control".into(),
                        ));
                    }
                }

                self.verify_owner() $($await)*?;

                let namespace = self.migration.owned_namespace()?;

                self.validate_schema(&namespace) $($await)*?;

                let history = self.migration.reserved(sql::HISTORY)?;
                let attempts = self.migration.reserved(sql::ATTEMPTS)?;
                let version = entry.version.to_string();
                let checksum = entry.checksum.as_bytes().as_slice();

                self.client
                    .execute(&sql::insert_history(&history), &[&version, &checksum])
                    $($await)*
                    .map_err(db)?;

                let count = self
                    .client
                    .execute(&sql::finish_attempt(&attempts, "applied"), &[&attempt])
                    $($await)*
                    .map_err(db)?;

                if count != 1 {
                    return Err(Error::History(
                        "attempt completion did not match a running attempt".into(),
                    ));
                }

                self.client.batch_execute("COMMIT") $($await)* .map_err(db)
            }

            /// Sends each statement of a script as its own request: several
            /// statements in one simple query share an implicit transaction,
            /// which statements such as `CREATE INDEX CONCURRENTLY` refuse. The
            /// first failure stops the script; earlier statements stay committed.
            $($async)* fn autocommit_script(&mut self, sql: &str) -> Result<(), Error<PgError>> {
                let statements = $crate::migrations::rupa_migrate::split_script(
                    sql,
                    $crate::migrations::rupa_core::DialectId::Postgres,
                )
                .map_err(|e| Error::Validation(e.into()))?;

                for statement in statements {
                    self.migration.dispatch();
                    self.client.batch_execute(&sql[statement]) $($await)* .map_err(db)?;
                }

                Ok(())
            }

            /// Typed queries in a migration are refused by the same grammar as
            /// scripts, since Postgres has no client-side authorizer.
            pub(crate) $($async)* fn execute_migration_statement(
                &mut self,
                statement: &$crate::migrations::rupa_core::ir::Statement,
                expect: $crate::migrations::rupa_core::query::Expect,
            ) -> Result<$crate::migrations::rupa_core::exec::Outcome, PgError> {
                self.migration.user_state().map_err(PgError::Migration)?;

                let rendered =
                    $crate::migrations::rupa_sql::render_cached(statement, &self.dialect)?;

                $crate::migrations::rupa_migrate::validate_script(
                    &rendered.sql,
                    $crate::migrations::rupa_core::DialectId::Postgres,
                )
                .map_err(|e| PgError::Migration(StateError::Script(e)))?;

                self.migration.dispatch();

                self.run(statement, expect) $($await)*
            }
        }
    };
}
