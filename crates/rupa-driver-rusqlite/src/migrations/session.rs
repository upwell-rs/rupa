use rupa_migrate::{CatalogEntry, Checksum, Error, HistoryEntry, Session, VersionScheme};

use super::{SqliteExecutor, db};
use crate::SqliteError;

impl Session for SqliteExecutor {
    fn acquire(&mut self, stream: &str, scheme: VersionScheme) -> Result<(), Error<SqliteError>> {
        if self.migration.owner.is_some()
            || self.migration.poisoned
            || !self.connection().is_autocommit()
        {
            return Err(Error::History(
                "session already owns a run or is not clean".into(),
            ));
        }

        self.validate_migration_connection()?;
        let timeout = self.migration.lock_timeout;
        self.connection().busy_timeout(timeout).map_err(db)?;
        self.connection()
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(db)?;
        match self.claim(stream, scheme) {
            Ok(token) => {
                self.migration.owner = Some(token);
                if let Err(error) = self.connection().execute_batch("COMMIT") {
                    self.migration.poisoned = true;

                    return Err(db(error));
                }

                Ok(())
            }
            Err(error) => {
                if self.connection().execute_batch("ROLLBACK").is_err() {
                    self.migration.poisoned = true;
                }

                Err(error)
            }
        }
    }

    fn history(&mut self) -> Result<Vec<HistoryEntry>, Error<SqliteError>> {
        if !self.has_metadata()? {
            return Ok(Vec::new());
        }

        let rows: Vec<(i64, String, Vec<u8>)> = self.connection().prepare("SELECT installation_rank, version, checksum FROM main._rupa_migration_history ORDER BY installation_rank").map_err(db)?.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(db)?.collect::<Result<_, _>>().map_err(db)?;

        rows.into_iter()
            .map(|(installation_rank, version, bytes)| {
                let installation_rank = u64::try_from(installation_rank)
                    .ok()
                    .filter(|rank| *rank > 0)
                    .ok_or_else(|| Error::History("invalid installation rank".into()))?;
                let bytes = bytes
                    .try_into()
                    .map_err(|_| Error::History("invalid stored checksum length".into()))?;

                Ok(HistoryEntry {
                    installation_rank,
                    version,
                    checksum: Checksum::from_bytes(bytes),
                })
            })
            .collect()
    }

    fn start_attempt(&mut self, entry: &CatalogEntry) -> Result<i64, Error<SqliteError>> {
        self.verify_owner()?;
        if self.migration.attempt.is_some() || !self.connection().is_autocommit() {
            return Err(Error::History(
                "a previous attempt remains unresolved".into(),
            ));
        }

        let result = self.connection().execute("INSERT INTO main._rupa_migration_attempts (version, checksum, status) VALUES (?1, ?2, 'running')", (entry.version.to_string(), entry.checksum.as_bytes().as_slice()));
        if let Err(error) = result {
            self.migration.poisoned = true;

            return Err(db(error));
        }

        let id = self.connection().last_insert_rowid();
        self.migration.attempt = Some(id);

        Ok(id)
    }

    fn begin_migration(&mut self) -> Result<(), Error<SqliteError>> {
        self.connection()
            .execute_batch("BEGIN IMMEDIATE")
            .map_err(db)?;
        self.verify_owner()
    }

    fn complete(&mut self, attempt: i64, entry: &CatalogEntry) -> Result<(), Error<SqliteError>> {
        self.verify_owner()?;
        self.validate_schema()?;
        if self.migration.attempt != Some(attempt) || self.connection().is_autocommit() {
            return Err(Error::History(
                "artifact transaction ended outside runner control".into(),
            ));
        }

        let result = (|| {
            self.connection()
                .execute(
                    "INSERT INTO main._rupa_migration_history (version, checksum) VALUES (?1, ?2)",
                    (
                        entry.version.to_string(),
                        entry.checksum.as_bytes().as_slice(),
                    ),
                )
                .map_err(db)?;
            let count = self.connection().execute("UPDATE main._rupa_migration_attempts SET status = 'applied', finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1 AND status = 'running'", [attempt]).map_err(db)?;
            if count != 1 {
                return Err(Error::History(
                    "attempt completion did not match a running attempt".into(),
                ));
            }

            self.connection().execute_batch("COMMIT").map_err(db)
        })();
        if result.is_err() {
            self.migration.poisoned = true;
        } else {
            self.migration.attempt = None;
        }

        result
    }

    fn abort(&mut self, attempt: i64) -> Result<(), Error<SqliteError>> {
        if !self.connection().is_autocommit()
            && let Err(error) = self.connection().execute_batch("ROLLBACK")
        {
            self.migration.poisoned = true;

            return Err(db(error));
        }

        self.verify_owner()?;
        let count = self.connection().execute("UPDATE main._rupa_migration_attempts SET status = 'failed', finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1 AND status = 'running'", [attempt]).map_err(db)?;
        if count != 1 || self.migration.attempt != Some(attempt) {
            self.migration.poisoned = true;

            return Err(Error::History("cannot reconcile failed attempt".into()));
        }

        self.migration.attempt = None;

        Ok(())
    }

    fn release(&mut self) -> Result<(), Error<SqliteError>> {
        self.verify_owner()?;
        if self.migration.attempt.is_some() || !self.connection().is_autocommit() {
            self.migration.poisoned = true;

            return Err(Error::History(
                "unresolved attempt/transaction; retaining durable owner claim".into(),
            ));
        }

        let owner = self.migration.owner.take().expect("verified owner");
        match self.connection().execute(
            "UPDATE main._rupa_migration_meta SET owner = NULL WHERE id = 1 AND owner = ?1",
            [&owner],
        ) {
            Ok(1) => Ok(()),
            result => {
                self.migration.owner = Some(owner);
                self.migration.poisoned = true;

                match result {
                    Err(error) => Err(db(error)),
                    _ => Err(Error::History("owner claim changed during release".into())),
                }
            }
        }
    }

    fn script(&mut self, sql: &str) -> Result<(), Error<SqliteError>> {
        self.user_executor()
            .map_err(Error::Database)?
            .executor
            .connection()
            .execute_batch(sql)
            .map_err(db)
    }
}
