//! The SQLite migration protocol's SQL. Reserved tables live in `main`.

pub const META: &str = "_rupa_migration_meta";
pub const HISTORY: &str = "_rupa_migration_history";
pub const ATTEMPTS: &str = "_rupa_migration_attempts";

/// Each reserved table's DDL, which `sqlite_schema` also reports verbatim.
pub const TABLES: [(&str, &str); 3] = [
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

/// The reserved objects present in `main`.
pub const RESERVED: &str =
    "SELECT name FROM main.sqlite_schema WHERE substr(lower(name), 1, 6) = '_rupa_'";

/// The stored DDL of table `?1`.
pub const TABLE_SQL: &str = "SELECT sql FROM main.sqlite_schema WHERE type = 'table' AND name = ?1";

/// Reserved objects besides the tables and their automatic indexes, or
/// objects on the reserved tables, such as triggers.
pub const EXTRA: &str = "SELECT count(*) FROM main.sqlite_schema WHERE (substr(lower(name), 1, 6) = '_rupa_' OR tbl_name IN (?1, ?2, ?3)) AND name NOT IN (?1, ?2, ?3) AND substr(name, 1, 17) != 'sqlite_autoindex_'";

/// Valid singleton headers for checksum format `?1`.
pub const HEADERS: &str = "SELECT count(*) FROM main._rupa_migration_meta WHERE schema_version = 1 AND checksum_format = ?1";

/// Each `AUTOINCREMENT` table and its largest key, which `sqlite_sequence` must match.
pub const SEQUENCES: [(&str, &str); 2] = [
    (
        HISTORY,
        "SELECT coalesce(max(installation_rank), 0) FROM main._rupa_migration_history",
    ),
    (
        ATTEMPTS,
        "SELECT coalesce(max(id), 0) FROM main._rupa_migration_attempts",
    ),
];

/// The `sqlite_sequence` rows of table `?1`.
pub const SEQUENCE: &str = "SELECT seq FROM main.sqlite_sequence WHERE name = ?1";

/// User objects in `main`, which make a target without metadata unmanaged.
pub const USER_OBJECTS: &str =
    "SELECT count(*) FROM main.sqlite_schema WHERE substr(lower(name), 1, 7) != 'sqlite_'";

pub const INSERT_HEADER: &str = "INSERT INTO main._rupa_migration_meta (id, schema_version, stream, scheme, checksum_format) VALUES (1, 1, ?1, ?2, ?3)";

/// The stored stream, scheme and owner claim.
pub const HEADER: &str = "SELECT stream, scheme, owner FROM main._rupa_migration_meta WHERE id = 1";

/// The oldest running attempt.
pub const UNRESOLVED: &str =
    "SELECT id FROM main._rupa_migration_attempts WHERE status = 'running' ORDER BY id LIMIT 1";

/// A fresh owner token.
pub const TOKEN: &str = "SELECT lower(hex(randomblob(16)))";

/// Claims an unowned target for token `?1`.
pub const CLAIM: &str =
    "UPDATE main._rupa_migration_meta SET owner = ?1 WHERE id = 1 AND owner IS NULL";

/// The stored owner claim.
pub const OWNER: &str = "SELECT owner FROM main._rupa_migration_meta WHERE id = 1";

/// Releases the claim of token `?1`.
pub const UNCLAIM: &str =
    "UPDATE main._rupa_migration_meta SET owner = NULL WHERE id = 1 AND owner = ?1";

pub const HISTORY_ROWS: &str = "SELECT installation_rank, version, checksum FROM main._rupa_migration_history ORDER BY installation_rank";

pub const INSERT_HISTORY: &str =
    "INSERT INTO main._rupa_migration_history (version, checksum) VALUES (?1, ?2)";

pub const INSERT_ATTEMPT: &str = "INSERT INTO main._rupa_migration_attempts (version, checksum, status) VALUES (?1, ?2, 'running')";

/// Finishes running attempt `?1` as `applied`.
pub const ATTEMPT_APPLIED: &str = "UPDATE main._rupa_migration_attempts SET status = 'applied', finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1 AND status = 'running'";

/// Finishes running attempt `?1` as `failed`.
pub const ATTEMPT_FAILED: &str = "UPDATE main._rupa_migration_attempts SET status = 'failed', finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now') WHERE id = ?1 AND status = 'running'";

/// `writable_schema`, which must be off.
pub const WRITABLE_SCHEMA: &str = "PRAGMA writable_schema";

/// Attached databases besides `main` and `temp`.
pub const ATTACHED: &str =
    "SELECT count(*) FROM pragma_database_list WHERE name NOT IN ('main', 'temp')";

/// Objects in the `temp` namespace.
pub const TEMPORARY: &str = "SELECT count(*) FROM temp.sqlite_schema";
