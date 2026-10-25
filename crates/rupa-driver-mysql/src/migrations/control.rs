use mysql::prelude::Queryable;
use rupa_migrate::{CHECKSUM_FORMAT, CatalogEntry, Error, VersionScheme};

use super::{db, sql};
use crate::{MySqlError, MySqlExecutor};

impl MySqlExecutor {
    /// Takes the named session lock, waiting at most the lock timeout.
    /// Named locks survive commits and end with the session.
    pub(super) fn lock(&mut self, name: &str) -> Result<(), Error<MySqlError>> {
        let acquired: Option<Option<i64>> = self
            .conn
            .exec_first(
                "SELECT GET_LOCK(?, ?)",
                (name, self.migration.lock_seconds()),
            )
            .map_err(db)?;

        match acquired.flatten() {
            Some(1) => Ok(()),
            Some(0) => Err(Error::Busy),
            _ => Err(Error::History(
                "GET_LOCK failed for the migration lock".into(),
            )),
        }
    }

    /// Restores the session after a failed acquisition; a lock or mode that
    /// cannot be restored poisons the connection, which must then be closed.
    pub(super) fn release_after_failure(&mut self, name: &str, sql_mode: &str) {
        let restored = self.conn.exec_drop("SET SESSION sql_mode = ?", (sql_mode,));
        let released: Result<Option<Option<i64>>, _> =
            self.conn.exec_first("SELECT RELEASE_LOCK(?)", (name,));

        if restored.is_err() || !matches!(released, Ok(Some(Some(1)))) {
            self.migration.poisoned = true;
        }
    }

    /// Whether the database holds reserved tables, which must then be valid metadata.
    pub(super) fn has_metadata(&mut self) -> Result<bool, Error<MySqlError>> {
        let count: Option<i64> = self.conn.query_first(sql::RESERVED_COUNT).map_err(db)?;

        if count == Some(0) {
            return Ok(false);
        }

        self.validate_schema()?;

        Ok(true)
    }

    pub(super) fn validate_schema(&mut self) -> Result<(), Error<MySqlError>> {
        for (table, _, expected) in sql::TABLES {
            let actual: Option<String> =
                self.conn.exec_first(sql::SIGNATURE, (table,)).map_err(db)?;

            if actual.as_deref() != Some(expected) {
                return Err(Error::History(format!(
                    "reserved metadata table {table} has an incompatible schema"
                )));
            }
        }

        let row: Option<(i64, i64, i64, i64)> = self
            .conn
            .exec_first(sql::INTEGRITY, (CHECKSUM_FORMAT,))
            .map_err(db)?;

        if row != Some((0, 0, 0, 1)) {
            return Err(Error::History(
                "metadata ownership/signature or singleton header is invalid".into(),
            ));
        }

        Ok(())
    }

    /// Classifies the database and creates metadata on a fresh target, then
    /// checks the stream and outstanding attempts, under the named lock.
    ///
    /// MySQL commits each `CREATE TABLE` implicitly, so creation is not
    /// atomic: an interrupted bootstrap leaves partial metadata, which the
    /// next run refuses as incompatible instead of completing it.
    pub(super) fn bootstrap(
        &mut self,
        stream: &str,
        scheme: VersionScheme,
    ) -> Result<(), Error<MySqlError>> {
        if !self.has_metadata()? {
            let objects: Option<i64> = self.conn.query_first(sql::USER_OBJECTS).map_err(db)?;

            if objects != Some(0) {
                return Err(Error::Unmanaged);
            }

            for (_, ddl, _) in sql::TABLES {
                self.conn.query_drop(ddl).map_err(db)?;
            }

            self.conn
                .exec_drop(
                    sql::INSERT_HEADER,
                    (stream, scheme.as_str(), CHECKSUM_FORMAT),
                )
                .map_err(db)?;
            self.validate_schema()?;
        }

        let header: Option<(String, String, Option<i64>)> =
            self.conn.query_first(sql::HEADER).map_err(db)?;
        let Some((stored_stream, stored_scheme, unresolved)) = header else {
            return Err(Error::History("missing migration metadata header".into()));
        };

        if stored_stream != stream || stored_scheme != scheme.as_str() {
            return Err(Error::History(
                "stream or version scheme differs from managed history".into(),
            ));
        }

        if let Some(attempt) = unresolved {
            return Err(Error::Uncertain(attempt));
        }

        Ok(())
    }

    /// Commits history and attempt completion in one InnoDB transaction,
    /// after the body's effects already committed.
    pub(super) fn checked_completion(
        &mut self,
        attempt: i64,
        entry: &CatalogEntry,
    ) -> Result<(), Error<MySqlError>> {
        if self.migration.attempt != Some(attempt) || !self.migration.body {
            return Err(Error::History(
                "attempt completion did not match the running attempt".into(),
            ));
        }

        self.verify_owner()?;
        self.validate_schema()?;
        self.conn.query_drop("START TRANSACTION").map_err(db)?;
        self.conn
            .exec_drop(
                sql::INSERT_HISTORY,
                (
                    entry.version.to_string(),
                    entry.checksum.as_bytes().as_slice(),
                ),
            )
            .map_err(db)?;
        self.conn
            .exec_drop(sql::ATTEMPT_APPLIED, (attempt,))
            .map_err(db)?;

        if self.conn.affected_rows() != 1 {
            return Err(Error::History(
                "attempt completion did not match a running attempt".into(),
            ));
        }

        self.conn.query_drop("COMMIT").map_err(db)
    }
}
