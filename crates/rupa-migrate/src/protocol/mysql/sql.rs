//! The MySQL migration protocol's SQL. Reserved tables live in the selected
//! database, which is the owned namespace.

pub const META: &str = "_rupa_migration_meta";
pub const HISTORY: &str = "_rupa_migration_history";
pub const ATTEMPTS: &str = "_rupa_migration_attempts";

/// Each reserved table's DDL and the column signature [`SIGNATURE`] reports for it.
pub const TABLES: [(&str, &str, &str); 3] = [
    (
        META,
        "CREATE TABLE _rupa_migration_meta (id INT PRIMARY KEY CHECK (id = 1), schema_version INT NOT NULL CHECK (schema_version = 1), stream VARCHAR(255) NOT NULL, scheme VARCHAR(64) NOT NULL, checksum_format VARCHAR(64) NOT NULL) ENGINE=InnoDB",
        "id int not null, schema_version int not null, stream varchar(255) not null, scheme varchar(64) not null, checksum_format varchar(64) not null",
    ),
    (
        HISTORY,
        "CREATE TABLE _rupa_migration_history (installation_rank BIGINT PRIMARY KEY CHECK (installation_rank > 0), version VARCHAR(255) NOT NULL UNIQUE, checksum BINARY(32) NOT NULL, applied_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6)) ENGINE=InnoDB",
        "installation_rank bigint not null, version varchar(255) not null, checksum binary(32) not null, applied_at datetime(6) not null",
    ),
    (
        ATTEMPTS,
        "CREATE TABLE _rupa_migration_attempts (id BIGINT AUTO_INCREMENT PRIMARY KEY, version VARCHAR(255) NOT NULL, checksum BINARY(32) NOT NULL, status VARCHAR(16) NOT NULL CHECK (status IN ('running', 'applied', 'failed')), started_at DATETIME(6) NOT NULL DEFAULT CURRENT_TIMESTAMP(6), finished_at DATETIME(6) NULL) ENGINE=InnoDB",
        "id bigint not null, version varchar(255) not null, checksum binary(32) not null, status varchar(16) not null, started_at datetime(6) not null, finished_at datetime(6)",
    ),
];

/// One row describing the connection: autocommit, the selected database and
/// the session `sql_mode`.
pub const DESCRIBE: &str = "SELECT @@SESSION.autocommit, DATABASE(), @@SESSION.sql_mode";

/// Fails with [`IN_TRANSACTION`] while any transaction is active, including
/// read-only snapshots `information_schema.innodb_trx` does not list.
/// Otherwise it only makes the next transaction read-write, the default.
pub const TRANSACTION_PROBE: &str = "SET TRANSACTION READ WRITE";

/// The SQLSTATE of `ER_CANT_CHANGE_TX_CHARACTERISTICS` (1568), raised by
/// [`TRANSACTION_PROBE`].
pub const IN_TRANSACTION: &str = "25001";

/// The column signature of reserved table `?` in the selected database.
pub const SIGNATURE: &str = "SELECT COALESCE(GROUP_CONCAT(CONCAT(LOWER(column_name), ' ', LOWER(column_type), IF(is_nullable = 'NO', ' not null', '')) ORDER BY ordinal_position SEPARATOR ', '), '') FROM information_schema.columns WHERE table_schema = DATABASE() AND table_name = ?";

/// Reserved relations present in the selected database.
pub const RESERVED_COUNT: &str = "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = DATABASE() AND LEFT(LOWER(table_name), 6) = '_rupa_'";

/// Unexpected reserved tables, non-InnoDB reserved tables, triggers on them,
/// and valid singleton headers.
pub const INTEGRITY: &str = "SELECT (SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = DATABASE() AND LEFT(LOWER(table_name), 6) = '_rupa_' AND table_name NOT IN ('_rupa_migration_meta', '_rupa_migration_history', '_rupa_migration_attempts')), (SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = DATABASE() AND table_name IN ('_rupa_migration_meta', '_rupa_migration_history', '_rupa_migration_attempts') AND engine <> 'InnoDB'), (SELECT COUNT(*) FROM information_schema.triggers WHERE event_object_schema = DATABASE() AND LEFT(LOWER(event_object_table), 6) = '_rupa_'), (SELECT COUNT(*) FROM _rupa_migration_meta WHERE schema_version = 1 AND checksum_format = ?)";

/// User tables, views, routines and events, which make a database without
/// metadata unmanaged.
pub const USER_OBJECTS: &str = "SELECT (SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = DATABASE()) + (SELECT COUNT(*) FROM information_schema.routines WHERE routine_schema = DATABASE()) + (SELECT COUNT(*) FROM information_schema.events WHERE event_schema = DATABASE())";

pub const INSERT_HEADER: &str = "INSERT INTO _rupa_migration_meta (id, schema_version, stream, scheme, checksum_format) VALUES (1, 1, ?, ?, ?)";

/// The stored stream, scheme and oldest running attempt.
pub const HEADER: &str = "SELECT stream, scheme, (SELECT MIN(id) FROM _rupa_migration_attempts WHERE status = 'running') FROM _rupa_migration_meta WHERE id = 1";

pub const HISTORY_ROWS: &str = "SELECT installation_rank, version, checksum FROM _rupa_migration_history ORDER BY installation_rank";

/// Records a completed artifact with the next installation rank. The derived
/// table lets MySQL read the table it inserts into.
pub const INSERT_HISTORY: &str = "INSERT INTO _rupa_migration_history (installation_rank, version, checksum) SELECT next_rank, ?, ? FROM (SELECT COALESCE(MAX(installation_rank), 0) + 1 AS next_rank FROM _rupa_migration_history) AS ranks";

pub const INSERT_ATTEMPT: &str =
    "INSERT INTO _rupa_migration_attempts (version, checksum, status) VALUES (?, ?, 'running')";

/// Finishes running attempt `?` as `applied`.
pub const ATTEMPT_APPLIED: &str = "UPDATE _rupa_migration_attempts SET status = 'applied', finished_at = CURRENT_TIMESTAMP(6) WHERE id = ? AND status = 'running'";

/// Finishes running attempt `?` as `failed`, before any of its body ran.
pub const ATTEMPT_FAILED: &str = "UPDATE _rupa_migration_attempts SET status = 'failed', finished_at = CURRENT_TIMESTAMP(6) WHERE id = ? AND status = 'running'";

/// Whether this session holds the named lock `?`.
pub const HOLDS_LOCK: &str = "SELECT COALESCE(IS_USED_LOCK(?) = CONNECTION_ID(), 0)";
