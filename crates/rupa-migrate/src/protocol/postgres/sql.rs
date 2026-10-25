//! The PostgreSQL migration protocol's SQL, shared by both drivers.

pub const META: &str = "_rupa_migration_meta";
pub const HISTORY: &str = "_rupa_migration_history";
pub const ATTEMPTS: &str = "_rupa_migration_attempts";

/// Every reserved relation the tables below create, including their
/// constraint indexes and identity sequence.
pub const RELATIONS: [&str; 8] = [
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
pub const TABLES: [(&str, &str, &str); 3] = [
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

pub const SIGNATURE: &str = "SELECT coalesce(string_agg(a.attname || ' ' || format_type(a.atttypid, a.atttypmod) || CASE WHEN a.attnotnull THEN ' not null' ELSE '' END, ', ' ORDER BY a.attnum), '') FROM pg_attribute a JOIN pg_class c ON c.oid = a.attrelid JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind = 'r' AND a.attnum > 0 AND NOT a.attisdropped";

/// One row describing the connection: autocommit, the string setting, the
/// database and the current schema. Sent with the simple query protocol,
/// where an implicit transaction starts with its statement.
pub const DESCRIBE: &str = "SELECT transaction_timestamp() = statement_timestamp(), current_setting('standard_conforming_strings'), current_database(), current_schema()";

/// Whether this session holds the advisory lock split into `$1`/`$2`.
pub const HOLDS_LOCK: &str = "SELECT EXISTS (SELECT 1 FROM pg_locks WHERE locktype = 'advisory' AND pid = pg_backend_pid() AND granted AND objsubid = 1 AND classid::bigint = $1 AND objid::bigint = $2)";

/// Reserved relations present in namespace `$1`.
pub const RESERVED_COUNT: &str = "SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = $1 AND left(lower(c.relname), 6) = '_rupa_'";

/// User objects in namespace `$1`, which make a target without metadata unmanaged.
pub const USER_OBJECTS: &str = "SELECT (SELECT count(*) FROM pg_class c WHERE c.relnamespace = n.oid) + (SELECT count(*) FROM pg_proc p WHERE p.pronamespace = n.oid) + (SELECT count(*) FROM pg_type t WHERE t.typnamespace = n.oid AND t.typtype <> 'c' AND t.typcategory <> 'A') FROM pg_namespace n WHERE n.nspname = $1";

/// Unexpected reserved relations, triggers on metadata tables, and the
/// number of valid singleton headers, in namespace `ns`.
pub fn integrity(ns: &str) -> String {
    format!(
        "SELECT (SELECT count(*) FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = $1 AND left(lower(c.relname), 6) = '_rupa_' AND c.relname <> ALL($2)), (SELECT count(*) FROM pg_trigger t JOIN pg_class c ON c.oid = t.tgrelid JOIN pg_namespace n ON n.oid = c.relnamespace WHERE n.nspname = $1 AND c.relname = ANY($3) AND NOT t.tgisinternal), (SELECT count(*) FROM {ns}.{META} WHERE schema_version = 1 AND checksum_format = $4)"
    )
}

/// Inserts the singleton header into `ns`.
pub fn insert_header(ns: &str) -> String {
    format!(
        "INSERT INTO {ns}.{META} (id, schema_version, stream, scheme, checksum_format) VALUES (1, 1, $1, $2, $3)"
    )
}

/// The stored stream, scheme and oldest running attempt in `ns`.
pub fn header(ns: &str) -> String {
    format!(
        "SELECT stream, scheme, (SELECT min(id) FROM {ns}.{ATTEMPTS} WHERE status = 'running') FROM {ns}.{META} WHERE id = 1"
    )
}

/// Installed history in `ns`, in installation order.
pub fn history(ns: &str) -> String {
    format!(
        "SELECT installation_rank, version, checksum FROM {ns}.{HISTORY} ORDER BY installation_rank"
    )
}

/// Records a completed artifact with the next installation rank.
pub fn insert_history(history: &str) -> String {
    format!(
        "INSERT INTO {history} (installation_rank, version, checksum) SELECT coalesce(max(installation_rank), 0) + 1, $1, $2 FROM {history}"
    )
}

/// Journals a running attempt, returning its id.
pub fn insert_attempt(attempts: &str) -> String {
    format!(
        "INSERT INTO {attempts} (version, checksum, status) VALUES ($1, $2, 'running') RETURNING id"
    )
}

/// Finishes running attempt `$1` with `status`.
pub fn finish_attempt(attempts: &str, status: &str) -> String {
    format!(
        "UPDATE {attempts} SET status = '{status}', finished_at = clock_timestamp() WHERE id = $1 AND status = 'running'"
    )
}
