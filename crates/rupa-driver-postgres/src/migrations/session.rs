use postgres::error::SqlState;
use rupa_migrate::sync::Session;
use rupa_migrate::{CatalogEntry, Checksum, Error, HistoryEntry, VersionScheme};

use super::schema::{ATTEMPTS, HISTORY};
use super::{Owner, PgExecutor, db, quote};
use crate::PgError;

/// PostgreSQL's largest `lock_timeout`; longer waits are clamped to it.
const MAX_LOCK_TIMEOUT_MS: u128 = i32::MAX as u128;

impl PgExecutor {
    /// Takes the session advisory lock, waiting at most the lock timeout.
    /// Session locks survive the transaction that bounds the wait.
    fn lock(&mut self, key: i64) -> Result<(), Error<PgError>> {
        let timeout = self.migration.lock_timeout;
        let millis = timeout.as_millis().clamp(1, MAX_LOCK_TIMEOUT_MS);

        if timeout.is_zero() {
            let acquired: bool = self
                .client
                .query_one("SELECT pg_try_advisory_lock($1)", &[&key])
                .map_err(db)?
                .get(0);

            return if acquired { Ok(()) } else { Err(Error::Busy) };
        }

        if let Err(error) = self
            .client
            .batch_execute(&format!("BEGIN; SET LOCAL lock_timeout = '{millis}ms'"))
        {
            if self.client.batch_execute("ROLLBACK").is_err() {
                self.migration.poisoned = true;
            }

            return Err(db(error));
        }

        if let Err(error) = self.client.execute("SELECT pg_advisory_lock($1)", &[&key]) {
            let busy = error.code() == Some(&SqlState::LOCK_NOT_AVAILABLE);

            if self.client.batch_execute("ROLLBACK").is_err() {
                self.migration.poisoned = true;
            }

            return Err(if busy { Error::Busy } else { db(error) });
        }

        self.client.batch_execute("COMMIT").map_err(|error| {
            self.migration.poisoned = true;

            db(error)
        })
    }

    /// Releases the lock after a failed acquisition; a lock that cannot be
    /// released poisons the connection, which must then be closed.
    fn unlock_after_failure(&mut self, key: i64) {
        if self.client.batch_execute("ROLLBACK").is_err() {
            self.migration.poisoned = true;
        }

        let released = self
            .client
            .query_one("SELECT pg_advisory_unlock($1)", &[&key])
            .map(|row| row.get::<_, bool>(0));

        if !matches!(released, Ok(true)) {
            self.migration.poisoned = true;
        }
    }

    fn current_transaction(&mut self) -> Result<i64, Error<PgError>> {
        Ok(self
            .client
            .query_one("SELECT txid_current()", &[])
            .map_err(db)?
            .get(0))
    }

    fn record_completion(
        &mut self,
        attempt: i64,
        entry: &CatalogEntry,
    ) -> Result<(), Error<PgError>> {
        let history = self.reserved(HISTORY)?;
        let attempts = self.reserved(ATTEMPTS)?;

        self.client
            .execute(
                &format!("INSERT INTO {history} (installation_rank, version, checksum) SELECT coalesce(max(installation_rank), 0) + 1, $1, $2 FROM {history}"),
                &[&entry.version.to_string(), &entry.checksum.as_bytes().as_slice()],
            )
            .map_err(db)?;

        let count = self
            .client
            .execute(
                &format!("UPDATE {attempts} SET status = 'applied', finished_at = clock_timestamp() WHERE id = $1 AND status = 'running'"),
                &[&attempt],
            )
            .map_err(db)?;

        if count != 1 {
            return Err(Error::History(
                "attempt completion did not match a running attempt".into(),
            ));
        }

        self.client.batch_execute("COMMIT").map_err(db)
    }
}

impl Session for PgExecutor {
    fn acquire(&mut self, stream: &str, scheme: VersionScheme) -> Result<(), Error<PgError>> {
        if self.migration.owner.is_some() || self.migration.poisoned {
            return Err(Error::History(
                "session already owns a run or is not clean".into(),
            ));
        }

        let connection = self.validate_migration_connection()?;
        let key = Self::target_key(&connection);

        self.lock(key)?;

        let bootstrapped = self
            .client
            .batch_execute("BEGIN")
            .map_err(db)
            .and_then(|()| self.bootstrap(&connection.namespace, stream, scheme))
            .and_then(|()| self.client.batch_execute("COMMIT").map_err(db));

        if let Err(error) = bootstrapped {
            self.unlock_after_failure(key);

            return Err(error);
        }

        self.migration.owner = Some(Owner {
            key,
            namespace: connection.namespace,
        });

        Ok(())
    }

    fn history(&mut self) -> Result<Vec<HistoryEntry>, Error<PgError>> {
        let namespace = match &self.migration.owner {
            Some(owner) => owner.namespace.clone(),
            None => self.validate_migration_connection()?.namespace,
        };

        if !self.has_metadata(&namespace)? {
            return Ok(Vec::new());
        }

        let rows = self
            .client
            .query(
                &format!(
                    "SELECT installation_rank, version, checksum FROM {}.{HISTORY} ORDER BY installation_rank",
                    quote(&namespace)
                ),
                &[],
            )
            .map_err(db)?;

        rows.into_iter()
            .map(|row| {
                let installation_rank = u64::try_from(row.get::<_, i64>(0))
                    .ok()
                    .filter(|rank| *rank > 0)
                    .ok_or_else(|| Error::History("invalid installation rank".into()))?;
                let bytes = row
                    .get::<_, Vec<u8>>(2)
                    .try_into()
                    .map_err(|_| Error::History("invalid stored checksum length".into()))?;

                Ok(HistoryEntry {
                    installation_rank,
                    version: row.get(1),
                    checksum: Checksum::from_bytes(bytes),
                })
            })
            .collect()
    }

    fn start_attempt(&mut self, entry: &CatalogEntry) -> Result<i64, Error<PgError>> {
        self.verify_owner()?;

        if self.migration.attempt.is_some() || self.migration.transaction.is_some() {
            return Err(Error::History(
                "a previous attempt remains unresolved".into(),
            ));
        }

        let attempts = self.reserved(ATTEMPTS)?;
        let id: i64 = self
            .client
            .query_one(
                &format!("INSERT INTO {attempts} (version, checksum, status) VALUES ($1, $2, 'running') RETURNING id"),
                &[&entry.version.to_string(), &entry.checksum.as_bytes().as_slice()],
            )
            .map_err(|error| {
                self.migration.poisoned = true;

                db(error)
            })?
            .get(0);

        self.migration.attempt = Some(id);

        Ok(id)
    }

    fn begin_migration(&mut self) -> Result<(), Error<PgError>> {
        self.statements.clear();
        self.client.batch_execute("BEGIN").map_err(db)?;
        self.migration.transaction = Some(self.current_transaction()?);

        self.verify_owner()
    }

    /// Commits only if the body left the artifact transaction it was given:
    /// the transaction id proves no statement committed or replaced it.
    fn complete(&mut self, attempt: i64, entry: &CatalogEntry) -> Result<(), Error<PgError>> {
        let result = (|| {
            self.verify_owner()?;

            let namespace = self.owned_namespace()?;

            self.validate_schema(&namespace)?;

            let transaction = self.migration.transaction;

            if self.migration.attempt != Some(attempt)
                || transaction.is_none()
                || transaction != Some(self.current_transaction()?)
            {
                return Err(Error::History(
                    "artifact transaction ended outside runner control".into(),
                ));
            }

            self.record_completion(attempt, entry)
        })();

        if result.is_err() {
            self.migration.poisoned = true;
        } else {
            self.migration.attempt = None;
            self.migration.transaction = None;
        }

        result
    }

    fn abort(&mut self, attempt: i64) -> Result<(), Error<PgError>> {
        if self.migration.transaction.take().is_some()
            && let Err(error) = self.client.batch_execute("ROLLBACK")
        {
            self.migration.poisoned = true;

            return Err(db(error));
        }

        self.verify_owner()?;

        let attempts = self.reserved(ATTEMPTS)?;
        let count = self
            .client
            .execute(
                &format!("UPDATE {attempts} SET status = 'failed', finished_at = clock_timestamp() WHERE id = $1 AND status = 'running'"),
                &[&attempt],
            )
            .map_err(db)?;

        if count != 1 || self.migration.attempt != Some(attempt) {
            self.migration.poisoned = true;

            return Err(Error::History("cannot reconcile failed attempt".into()));
        }

        self.migration.attempt = None;

        Ok(())
    }

    fn release(&mut self) -> Result<(), Error<PgError>> {
        self.verify_owner()?;

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
        let released: bool = self
            .client
            .query_one("SELECT pg_advisory_unlock($1)", &[&key])
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

    fn script(&mut self, sql: &str) -> Result<(), Error<PgError>> {
        self.user_state()
            .map_err(|state| Error::Database(PgError::Migration(state)))?;

        let result = self.client.batch_execute(sql).map_err(db);

        self.statements.clear();

        result
    }
}
