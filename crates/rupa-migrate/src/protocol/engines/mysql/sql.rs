//! The MySQL migration protocol's SQL, over the journal table that
//! `sql/mysql.sql` defines in the selected database, the owned namespace.

/// The journal table.
pub const JOURNAL: &str = "_rupa_migrations";

/// The journal's definition.
pub const CREATE: &str = include_str!("../../../../sql/mysql.sql");

/// The journal's columns, as [`SIGNATURE`] reports them.
pub const COLUMNS: &str = "id bigint not null, kind varchar(16) not null, version varchar(255), checksum binary(32), policy varchar(16), status varchar(16), reason text, evidence text, started_at datetime(6) not null, finished_at datetime(6), schema_version int, stream varchar(255), scheme varchar(64), checksum_format varchar(64), owner varchar(64)";

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

/// The column signature of table `?` in the selected database.
pub const SIGNATURE: &str = "
    SELECT COALESCE(GROUP_CONCAT(
        CONCAT(LOWER(column_name), ' ', LOWER(column_type), IF(is_nullable = 'NO', ' not null', ''))
        ORDER BY ordinal_position SEPARATOR ', '
    ), '')
    FROM information_schema.columns
    WHERE table_schema = DATABASE() AND table_name = ?";

/// Reserved tables present in the selected database.
pub const RESERVED_COUNT: &str = "
    SELECT COUNT(*)
    FROM information_schema.tables
    WHERE table_schema = DATABASE() AND LEFT(LOWER(table_name), 6) = '_rupa_'";

/// Unexpected reserved tables, a non-InnoDB journal, triggers on reserved
/// tables, header rows, and header rows for checksum format `?`.
pub const INTEGRITY: &str = "
    SELECT
        (SELECT COUNT(*) FROM information_schema.tables
         WHERE table_schema = DATABASE() AND LEFT(LOWER(table_name), 6) = '_rupa_'
           AND table_name <> '_rupa_migrations'),
        (SELECT COUNT(*) FROM information_schema.tables
         WHERE table_schema = DATABASE() AND table_name = '_rupa_migrations'
           AND engine <> 'InnoDB'),
        (SELECT COUNT(*) FROM information_schema.triggers
         WHERE event_object_schema = DATABASE()
           AND LEFT(LOWER(event_object_table), 6) = '_rupa_'),
        (SELECT COUNT(*) FROM _rupa_migrations WHERE kind = 'init'),
        (SELECT COUNT(*) FROM _rupa_migrations WHERE kind = 'init' AND checksum_format = ?)";

/// User tables, views, routines and events, which make a database without a
/// journal unmanaged.
pub const USER_OBJECTS: &str = "
    SELECT (SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = DATABASE())
         + (SELECT COUNT(*) FROM information_schema.routines WHERE routine_schema = DATABASE())
         + (SELECT COUNT(*) FROM information_schema.events WHERE event_schema = DATABASE())";

/// Inserts the header row: stream, scheme, checksum format.
pub const INSERT_HEADER: &str = "
    INSERT INTO _rupa_migrations (kind, schema_version, stream, scheme, checksum_format)
    VALUES ('init', 1, ?, ?, ?)";

/// The stored stream and scheme, and the oldest running attempt. MySQL
/// journals only nontransactional attempts, so every running one is unresolved.
pub const HEADER: &str = "
    SELECT stream, scheme,
           (SELECT MIN(id) FROM _rupa_migrations WHERE kind = 'up' AND status = 'running')
    FROM _rupa_migrations
    WHERE kind = 'init'";

/// Installed history, in installation order.
pub const HISTORY: &str = "
    SELECT id, version, checksum, status
    FROM _rupa_migrations
    WHERE kind = 'up' AND status IN ('applied', 'recovered')
    ORDER BY id";

/// Journals a running attempt: version, checksum, policy.
pub const INSERT_ATTEMPT: &str = "
    INSERT INTO _rupa_migrations (kind, version, checksum, policy, status)
    VALUES ('up', ?, ?, ?, 'running')";

/// Finishes a running attempt with a status, reason and evidence; the
/// attempt id is bound last.
pub const FINISH: &str = "
    UPDATE _rupa_migrations
    SET status = ?, reason = ?, evidence = ?, finished_at = CURRENT_TIMESTAMP(6)
    WHERE id = ? AND kind = 'up' AND status = 'running'";

/// Attempt `?`, locked for its resolution.
pub const ATTEMPT: &str = "
    SELECT version, checksum, status
    FROM _rupa_migrations
    WHERE id = ? AND kind = 'up'
    FOR UPDATE";

/// Whether this session holds the named lock `?`.
pub const HOLDS_LOCK: &str = "SELECT COALESCE(IS_USED_LOCK(?) = CONNECTION_ID(), 0)";
