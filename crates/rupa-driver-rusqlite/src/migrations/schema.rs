use rupa_migrate::{CHECKSUM_FORMAT, Error, VersionScheme};
use rusqlite::OptionalExtension;

use super::{SqliteExecutor, db};
use crate::SqliteError;

pub(super) const META: &str = "_rupa_migration_meta";
pub(super) const HISTORY: &str = "_rupa_migration_history";
pub(super) const ATTEMPTS: &str = "_rupa_migration_attempts";

const TABLES: [(&str, &str); 3] = [
    (
        META,
        "CREATE TABLE _rupa_migration_meta (id INTEGER PRIMARY KEY CHECK (id = 1), schema_version INTEGER NOT NULL CHECK (schema_version = 1), stream TEXT NOT NULL, scheme TEXT NOT NULL, checksum_format TEXT NOT NULL, owner TEXT)",
    ),
    (
        HISTORY,
        "CREATE TABLE _rupa_migration_history (installation_rank INTEGER PRIMARY KEY AUTOINCREMENT, version TEXT NOT NULL UNIQUE, checksum BLOB NOT NULL CHECK (length(checksum) = 32), applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')))",
    ),
    (
        ATTEMPTS,
        "CREATE TABLE _rupa_migration_attempts (id INTEGER PRIMARY KEY AUTOINCREMENT, version TEXT NOT NULL, checksum BLOB NOT NULL CHECK (length(checksum) = 32), status TEXT NOT NULL CHECK (status IN ('running', 'applied', 'failed')), started_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')), finished_at TEXT)",
    ),
];

impl SqliteExecutor {
    pub(super) fn has_metadata(&mut self) -> Result<bool, Error<SqliteError>> {
        let names: Vec<String> = self
            .connection()
            .prepare(
                "SELECT name FROM main.sqlite_schema WHERE substr(lower(name), 1, 6) = '_rupa_'",
            )
            .map_err(db)?
            .query_map([], |r| r.get(0))
            .map_err(db)?
            .collect::<Result<_, _>>()
            .map_err(db)?;
        if names.is_empty() {
            return Ok(false);
        }

        self.validate_schema()?;

        Ok(true)
    }

    pub(super) fn validate_schema(&mut self) -> Result<(), Error<SqliteError>> {
        for (name, expected) in TABLES {
            let actual: Option<String> = self
                .connection()
                .query_row(
                    "SELECT sql FROM main.sqlite_schema WHERE type = 'table' AND name = ?1",
                    [name],
                    |r| r.get(0),
                )
                .optional()
                .map_err(db)?;
            if actual.as_deref() != Some(expected) {
                return Err(Error::History(format!(
                    "reserved metadata table {name} has an incompatible schema"
                )));
            }
        }

        let extra: i64 = self.connection().query_row("SELECT count(*) FROM main.sqlite_schema WHERE (substr(lower(name), 1, 6) = '_rupa_' OR tbl_name IN (?1, ?2, ?3)) AND name NOT IN (?1, ?2, ?3) AND substr(name, 1, 17) != 'sqlite_autoindex_'", [META, HISTORY, ATTEMPTS], |r| r.get(0)).map_err(db)?;
        let count: i64 = self.connection().query_row("SELECT count(*) FROM main._rupa_migration_meta WHERE schema_version = 1 AND checksum_format = ?1", [CHECKSUM_FORMAT], |r| r.get(0)).map_err(db)?;
        if extra != 0 || count != 1 {
            return Err(Error::History(
                "metadata ownership/signature or singleton header is invalid".into(),
            ));
        }

        for (table, maximum_query) in [
            (
                HISTORY,
                "SELECT coalesce(max(installation_rank), 0) FROM main._rupa_migration_history",
            ),
            (
                ATTEMPTS,
                "SELECT coalesce(max(id), 0) FROM main._rupa_migration_attempts",
            ),
        ] {
            let maximum: i64 = self
                .connection()
                .query_row(maximum_query, [], |row| row.get(0))
                .map_err(db)?;
            let sequences: Vec<i64> = self
                .connection()
                .prepare("SELECT seq FROM main.sqlite_sequence WHERE name = ?1")
                .map_err(db)?
                .query_map([table], |row| row.get(0))
                .map_err(db)?
                .collect::<Result<_, _>>()
                .map_err(db)?;
            if !(sequences.is_empty() && maximum == 0 || sequences.as_slice() == [maximum]) {
                return Err(Error::History(format!(
                    "reserved sequence for {table} was changed"
                )));
            }
        }

        Ok(())
    }

    pub(super) fn claim(
        &mut self,
        stream: &str,
        scheme: VersionScheme,
    ) -> Result<String, Error<SqliteError>> {
        if !self.has_metadata()? {
            let objects: i64 = self.connection().query_row("SELECT count(*) FROM main.sqlite_schema WHERE substr(lower(name), 1, 7) != 'sqlite_'", [], |r| r.get(0)).map_err(db)?;
            if objects != 0 {
                return Err(Error::Unmanaged);
            }

            for (_, sql) in TABLES {
                self.connection().execute_batch(sql).map_err(db)?;
            }

            self.connection().execute("INSERT INTO main._rupa_migration_meta (id, schema_version, stream, scheme, checksum_format) VALUES (1, 1, ?1, ?2, ?3)", (stream, scheme.as_str(), CHECKSUM_FORMAT)).map_err(db)?;
        }

        let (stored_stream, stored_scheme, owner): (String, String, Option<String>) = self
            .connection()
            .query_row(
                "SELECT stream, scheme, owner FROM main._rupa_migration_meta WHERE id = 1",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .map_err(db)?;
        if stored_stream != stream || stored_scheme != scheme.as_str() {
            return Err(Error::History(
                "stream or version scheme differs from managed history".into(),
            ));
        }

        if owner.is_some() {
            return Err(Error::Busy);
        }

        let unresolved: Option<i64> = self.connection().query_row("SELECT id FROM main._rupa_migration_attempts WHERE status = 'running' ORDER BY id LIMIT 1", [], |r| r.get(0)).optional().map_err(db)?;
        if let Some(attempt) = unresolved {
            return Err(Error::Uncertain(attempt));
        }

        let token: String = self
            .connection()
            .query_row("SELECT lower(hex(randomblob(16)))", [], |r| r.get(0))
            .map_err(db)?;
        self.connection()
            .execute(
                "UPDATE main._rupa_migration_meta SET owner = ?1 WHERE id = 1 AND owner IS NULL",
                [&token],
            )
            .map_err(db)?;

        Ok(token)
    }
}
