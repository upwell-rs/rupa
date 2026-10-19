/// The migration `Session` implementation for one mode, built on the
/// control steps of `__pg_migration_control!`.
#[doc(hidden)]
#[macro_export]
macro_rules! __pg_migration_session {
    ([$($async:tt)*] [$($await:tt)*] $session:path) => {
        impl $session for PgExecutor {
            $($async)* fn acquire(
                &mut self,
                stream: &str,
                scheme: VersionScheme,
            ) -> Result<(), Error<PgError>> {
                if self.migration.owner.is_some() || self.migration.poisoned {
                    return Err(Error::History(
                        "session already owns a run or is not clean".into(),
                    ));
                }

                let connection = self.describe_connection() $($await)*?;
                let key = connection.key();

                self.lock(key) $($await)*?;

                if let Err(error) = self.bootstrap(&connection.namespace, stream, scheme) $($await)* {
                    self.unlock_after_failure(key) $($await)*;

                    return Err(error);
                }

                self.migration.owner = Some(Owner {
                    key,
                    namespace: connection.namespace,
                });

                Ok(())
            }

            $($async)* fn history(&mut self) -> Result<Vec<HistoryEntry>, Error<PgError>> {
                let namespace = match &self.migration.owner {
                    Some(owner) => owner.namespace.clone(),
                    None => self.describe_connection() $($await)*?.namespace,
                };

                if !self.has_metadata(&namespace) $($await)*? {
                    return Ok(Vec::new());
                }

                let rows = self
                    .client
                    .query(&sql::history(&quote(&namespace)), &[])
                    $($await)*
                    .map_err(db)?;

                decode_history(rows)
            }

            $($async)* fn start_attempt(&mut self, entry: &CatalogEntry) -> Result<i64, Error<PgError>> {
                self.verify_owner() $($await)*?;

                if self.migration.attempt.is_some() || self.migration.transaction.is_some() {
                    return Err(Error::History(
                        "a previous attempt remains unresolved".into(),
                    ));
                }

                let attempts = self.migration.reserved(sql::ATTEMPTS)?;
                let version = entry.version.to_string();
                let checksum = entry.checksum.as_bytes().as_slice();
                let inserted = self
                    .client
                    .query_one(&sql::insert_attempt(&attempts), &[&version, &checksum])
                    $($await)*;
                let id: i64 = inserted
                    .map_err(|error| {
                        self.migration.poisoned = true;

                        db(error)
                    })?
                    .get(0);

                self.migration.attempt = Some(id);

                Ok(id)
            }

            $($async)* fn begin_migration(&mut self) -> Result<(), Error<PgError>> {
                self.statements.clear();
                self.client.batch_execute("BEGIN") $($await)* .map_err(db)?;
                self.migration.transaction = Some(self.current_transaction() $($await)*?);

                self.verify_owner() $($await)*
            }

            $($async)* fn complete(
                &mut self,
                attempt: i64,
                entry: &CatalogEntry,
            ) -> Result<(), Error<PgError>> {
                let result = self.checked_completion(attempt, entry) $($await)*;

                if result.is_err() {
                    self.migration.poisoned = true;
                } else {
                    self.migration.attempt = None;
                    self.migration.transaction = None;
                }

                result
            }

            $($async)* fn abort(&mut self, attempt: i64) -> Result<(), Error<PgError>> {
                if self.migration.transaction.take().is_some()
                    && let Err(error) = self.client.batch_execute("ROLLBACK") $($await)*
                {
                    self.migration.poisoned = true;

                    return Err(db(error));
                }

                self.verify_owner() $($await)*?;

                let attempts = self.migration.reserved(sql::ATTEMPTS)?;
                let count = self
                    .client
                    .execute(&sql::finish_attempt(&attempts, "failed"), &[&attempt])
                    $($await)*
                    .map_err(db)?;

                if count != 1 || self.migration.attempt != Some(attempt) {
                    self.migration.poisoned = true;

                    return Err(Error::History("cannot reconcile failed attempt".into()));
                }

                self.migration.attempt = None;

                Ok(())
            }

            $($async)* fn release(&mut self) -> Result<(), Error<PgError>> {
                self.verify_owner() $($await)*?;

                if self.migration.attempt.is_some() || self.migration.transaction.is_some() {
                    self.migration.poisoned = true;

                    return Err(Error::History(
                        "unresolved attempt/transaction; retaining the advisory lock".into(),
                    ));
                }

                let Some(key) = self.migration.owner.as_ref().map(|owner| owner.key) else {
                    return Err(Error::History(
                        "migration connection does not own the target".into(),
                    ));
                };
                let unlocked = self
                    .client
                    .query_one("SELECT pg_advisory_unlock($1)", &[&key])
                    $($await)*;
                let released: bool = unlocked
                    .map_err(|error| {
                        self.migration.poisoned = true;

                        db(error)
                    })?
                    .get(0);

                if !released {
                    self.migration.poisoned = true;

                    return Err(Error::History(
                        "advisory lock changed during release".into(),
                    ));
                }

                self.migration.owner = None;

                Ok(())
            }

            $($async)* fn script(&mut self, sql: &str) -> Result<(), Error<PgError>> {
                self.migration
                    .user_state()
                    .map_err(|state| Error::Database(PgError::Migration(state)))?;

                let result = self.client.batch_execute(sql) $($await)* .map_err(db);

                self.statements.clear();

                result
            }

            fn abandon(&mut self) {
                self.migration.poisoned = true;
            }
        }
    };
}
