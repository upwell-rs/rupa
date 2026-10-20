use mysql::prelude::Queryable;
use rupa_core::DialectId;
use rupa_core::tx::TxConnection;
use rupa_migrate::sync::Session;
use rupa_migrate::{CatalogEntry, Checksum, Error, HistoryEntry, Policy, VersionScheme};

use super::{Owner, db, sql};
use crate::{MySqlError, MySqlExecutor};

impl Session for MySqlExecutor {
    const POLICIES: &'static [Policy] = &[Policy::NonTransactional];

    fn acquire(&mut self, stream: &str, scheme: VersionScheme) -> Result<(), Error<MySqlError>> {
        if self.migration.owner.is_some() || self.migration.poisoned {
            return Err(Error::History(
                "session already owns a run or is not clean".into(),
            ));
        }

        let connection = self.describe_connection()?;
        let lock = connection.lock();

        self.lock(&lock)?;

        let prepared = self
            .conn
            .exec_drop(
                "SET SESSION sql_mode = ?",
                (connection.migration_sql_mode(),),
            )
            .map_err(db)
            .and_then(|()| self.bootstrap(stream, scheme));

        if let Err(error) = prepared {
            self.release_after_failure(&lock, &connection.sql_mode);

            return Err(error);
        }

        self.migration.owner = Some(Owner {
            lock,
            sql_mode: connection.sql_mode,
        });

        Ok(())
    }

    fn history(&mut self) -> Result<Vec<HistoryEntry>, Error<MySqlError>> {
        self.clean().map_err(Error::Database)?;

        if !self.has_metadata()? {
            return Ok(Vec::new());
        }

        let rows: Vec<(i64, String, Vec<u8>)> = self.conn.query(sql::HISTORY_ROWS).map_err(db)?;

        rows.into_iter()
            .map(|(rank, version, checksum)| {
                let installation_rank = u64::try_from(rank)
                    .ok()
                    .filter(|rank| *rank > 0)
                    .ok_or_else(|| Error::History("invalid installation rank".into()))?;
                let bytes = checksum
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

    fn start_attempt(&mut self, entry: &CatalogEntry) -> Result<i64, Error<MySqlError>> {
        self.verify_owner()?;

        if self.migration.attempt.is_some() || self.migration.body {
            return Err(Error::History(
                "a previous attempt remains unresolved".into(),
            ));
        }

        let inserted = self.conn.exec_drop(
            sql::INSERT_ATTEMPT,
            (
                entry.version.to_string(),
                entry.checksum.as_bytes().as_slice(),
            ),
        );

        if let Err(error) = inserted {
            self.migration.poisoned = true;

            return Err(db(error));
        }

        let id = i64::try_from(self.conn.last_insert_id())
            .map_err(|_| Error::History("invalid attempt id".into()))?;

        self.migration.attempt = Some(id);

        Ok(id)
    }

    /// The body is marked begun only after ownership is verified, so a
    /// failure here records a replayable failed attempt: no statement ran.
    fn begin_migration(&mut self, policy: Policy) -> Result<(), Error<MySqlError>> {
        if !Self::POLICIES.contains(&policy) {
            return Err(Error::History(format!(
                "MySQL migration sessions cannot run {policy:?} bodies"
            )));
        }

        self.verify_owner()?;
        self.migration.body = true;

        Ok(())
    }

    fn complete(&mut self, attempt: i64, entry: &CatalogEntry) -> Result<(), Error<MySqlError>> {
        let result = self.checked_completion(attempt, entry);

        if result.is_err() {
            self.migration.poisoned = true;
        } else {
            self.migration.attempt = None;
            self.migration.body = false;
            self.migration.dispatched = false;
        }

        result
    }

    /// Once a statement was dispatched its effects may persist, so the
    /// attempt stays durably unresolved rather than being recorded as failed,
    /// which would permit a replay. A body refused before dispatching any
    /// statement records an ordinary failure.
    fn abort(&mut self, attempt: i64) -> Result<(), Error<MySqlError>> {
        self.migration.body = false;

        if std::mem::take(&mut self.migration.dispatched) {
            self.migration.attempt = None;

            return Err(Error::Uncertain(attempt));
        }

        self.verify_owner()?;
        self.conn
            .exec_drop(sql::ATTEMPT_FAILED, (attempt,))
            .map_err(db)?;

        if self.conn.affected_rows() != 1 || self.migration.attempt != Some(attempt) {
            self.migration.poisoned = true;

            return Err(Error::History("cannot reconcile failed attempt".into()));
        }

        self.migration.attempt = None;

        Ok(())
    }

    fn release(&mut self) -> Result<(), Error<MySqlError>> {
        self.verify_owner()?;

        if self.migration.attempt.is_some() || self.migration.body {
            self.migration.poisoned = true;

            return Err(Error::History(
                "unresolved attempt; retaining the named lock".into(),
            ));
        }

        let Some(owner) = self.migration.owner.as_ref() else {
            return Err(Error::History(
                "migration connection does not own the target".into(),
            ));
        };
        let restored = self
            .conn
            .exec_drop("SET SESSION sql_mode = ?", (owner.sql_mode.as_str(),));
        let released = restored.and_then(|()| {
            self.conn
                .exec_first::<Option<i64>, _, _>("SELECT RELEASE_LOCK(?)", (owner.lock.as_str(),))
        });

        match released {
            Ok(Some(Some(1))) => {
                self.migration.owner = None;

                Ok(())
            }
            Ok(_) => {
                self.migration.poisoned = true;

                Err(Error::History("named lock changed during release".into()))
            }
            Err(error) => {
                self.migration.poisoned = true;

                Err(db(error))
            }
        }
    }

    /// Sends each statement as its own request: MySQL commits DDL implicitly,
    /// so the first failure stops the script and earlier statements stay.
    fn script(&mut self, sql: &str) -> Result<(), Error<MySqlError>> {
        self.migration
            .user_state()
            .map_err(|state| Error::Database(MySqlError::Migration(state)))?;

        let statements = rupa_migrate::split_script(sql, DialectId::MySql)
            .map_err(|e| Error::Validation(e.into()))?;

        for statement in statements {
            self.migration.dispatched = true;
            self.conn.query_drop(&sql[statement]).map_err(db)?;
        }

        Ok(())
    }

    fn abandon(&mut self) {
        self.migration.poisoned = true;
    }
}
