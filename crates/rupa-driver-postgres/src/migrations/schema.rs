use rupa_migrate::{CHECKSUM_FORMAT, Error, VersionScheme};

use super::{PgExecutor, db, quote};
use crate::PgError;

pub(super) const META: &str = "_rupa_migration_meta";
pub(super) const HISTORY: &str = "_rupa_migration_history";
pub(super) const ATTEMPTS: &str = "_rupa_migration_attempts";

/// Every reserved relation the tables below create, including their
/// constraint indexes and identity sequence.
const RELATIONS: [&str; 8] = [
    META,
    HISTORY,
    ATTEMPTS,
    "_rupa_migration_meta_pkey",
    "_rupa_migration_history_pkey",
    "_rupa_migration_history_version_key",
    "_rupa_migration_attempts_pkey",
    "_rupa_migration_attempts_id_seq",
];

/// Each table's DDL (with `{ns}` for the owned namespace) and the column
/// signature `pg_attribute` reports for it.
const TABLES: [(&str, &str, &str); 3] = [
    (
        META,
        "CREATE TABLE {ns}._rupa_migration_meta (id integer PRIMARY KEY CHECK (id = 1), schema_version integer NOT NULL CHECK (schema_version = 1), stream text NOT NULL, scheme text NOT NULL, checksum_format text NOT NULL)",
        "id integer not null, schema_version integer not null, stream text not null, scheme text not null, checksum_format text not null",
    ),
    (
        HISTORY,
        "CREATE TABLE {ns}._rupa_migration_history (installation_rank bigint PRIMARY KEY CHECK (installation_rank > 0), version text NOT NULL UNIQUE, checksum bytea NOT NULL CHECK (length(checksum) = 32), applied_at timestamptz NOT NULL DEFAULT clock_timestamp())",
        "installation_rank bigint not null, version text not null, checksum bytea not null, applied_at timestamp with time zone not null",
    ),
    (
        ATTEMPTS,
        "CREATE TABLE {ns}._rupa_migration_attempts (id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY, version text NOT NULL, checksum bytea NOT NULL CHECK (length(checksum) = 32), status text NOT NULL CHECK (status IN ('running', 'applied', 'failed')), started_at timestamptz NOT NULL DEFAULT clock_timestamp(), finished_at timestamptz)",
        "id bigint not null, version text not null, checksum bytea not null, status text not null, started_at timestamp with time zone not null, finished_at timestamp with time zone",
    ),
];

const SIGNATURE: &str = "SELECT coalesce(string_agg(a.attname || ' ' || format_type(a.atttypid, a.atttypmod) || CASE WHEN a.attnotnull THEN ' not null' ELSE '' END, ', ' ORDER BY a.attnum), '') FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind = 'r' AND a.attnum > 0 AND NOT a.attisdropped";

impl PgExecutor {
    /// Whether the namespace holds reserved objects, which must then be valid metadata.
    pub(super) fn has_metadata(&mut self, namespace: &str) -> Result<bool, Error<PgError>> {
        let count: i64 = self
            .client
            .query_one(
                "SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = $1 AND left(lower(c.relname), 6) = '_rupa_'",
                &[&namespace],
            )
            .map_err(db)?
            .get(0);

        if count == 0 {
            return Ok(false);
        }

        self.validate_schema(namespace)?;

        Ok(true)
    }

    pub(super) fn validate_schema(&mut self, namespace: &str) -> Result<(), Error<PgError>> {
        let relations = RELATIONS.as_slice();
        let tables = &RELATIONS[..3];

        for (table, _, expected) in TABLES {
            let actual: String = self
                .client
                .query_one(SIGNATURE, &[&namespace, &table])
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
                &format!(
                    "SELECT (SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = $1 AND left(lower(c.relname), 6) = '_rupa_' AND c.relname <> ALL($2)), (SELECT count(*) FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = $1 AND c.relname = ANY($3) AND NOT t.tgisinternal), (SELECT count(*) FROM {}.{META} WHERE schema_version = 1 AND checksum_format = $4)",
                    quote(namespace)
                ),
                &[&namespace, &relations, &tables, &CHECKSUM_FORMAT],
            )
            .map_err(db)?;
        let (extra, triggers, header): (i64, i64, i64) = (row.get(0), row.get(1), row.get(2));

        if extra != 0 || triggers != 0 || header != 1 {
            return Err(Error::History(
                "metadata ownership/signature or singleton header is invalid".into(),
            ));
        }

        Ok(())
    }

    /// Classifies the namespace and creates metadata on a fresh target, then
    /// checks the stream and outstanding attempts. Runs in one transaction
    /// under the advisory lock.
    pub(super) fn bootstrap(
        &mut self,
        namespace: &str,
        stream: &str,
        scheme: VersionScheme,
    ) -> Result<(), Error<PgError>> {
        let ns = quote(namespace);

        if !self.has_metadata(namespace)? {
            let objects: i64 = self
                .client
                .query_one(
                    "SELECT (SELECT count(*) FROM pg_class c WHERE c.relnamespace = n.oid) + (SELECT count(*) FROM pg_proc p WHERE p.pronamespace = n.oid) + (SELECT count(*) FROM pg_type t WHERE t.typnamespace = n.oid AND t.typtype <> 'c' AND t.typcategory <> 'A') FROM pg_namespace n WHERE n.nspname = $1",
                    &[&namespace],
                )
                .map_err(db)?
                .get(0);

            if objects != 0 {
                return Err(Error::Unmanaged);
            }

            for (_, sql, _) in TABLES {
                self.client
                    .batch_execute(&sql.replace("{ns}", &ns))
                    .map_err(db)?;
            }

            self.client
                .execute(
                    &format!("INSERT INTO {ns}.{META} (id, schema_version, stream, scheme, checksum_format) VALUES (1, 1, $1, $2, $3)"),
                    &[&stream, &scheme.as_str(), &CHECKSUM_FORMAT],
                )
                .map_err(db)?;
        }

        let row = self
            .client
            .query_one(
                &format!("SELECT stream, scheme, (SELECT min(id) FROM {ns}.{ATTEMPTS} WHERE status = 'running') FROM {ns}.{META} WHERE id = 1"),
                &[],
            )
            .map_err(db)?;
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

        Ok(())
    }
}
