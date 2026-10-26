//! The SQLite migration protocol's SQL, over the journal table that
//! `sql/sqlite.sql` defines in `main`.

/// The journal table.
pub const JOURNAL: &str = "_rupa_migrations";

/// The journal's definition, comments included.
pub const CREATE: &str = include_str!("../../../../sql/sqlite.sql");

/// The `CREATE TABLE` statement of [`CREATE`], as `sqlite_schema` stores it:
/// from its first keyword to its last token.
pub fn definition() -> &'static str {
    let start = CREATE.find("CREATE TABLE").unwrap_or(0);

    CREATE[start..].trim_end()
}

/// The reserved objects present in `main`.
pub const RESERVED: &str = "
    SELECT name
    FROM main.sqlite_schema
    WHERE substr(lower(name), 1, 6) = '_rupa_'";

/// The stored definition of table `?1`.
pub const TABLE_SQL: &str = "
    SELECT sql
    FROM main.sqlite_schema
    WHERE type = 'table' AND name = ?1";

/// Reserved objects besides the journal, or objects on the journal, such as
/// triggers. The journal has no automatic indexes.
pub const EXTRA: &str = "
    SELECT count(*)
    FROM main.sqlite_schema
    WHERE (substr(lower(name), 1, 6) = '_rupa_' OR tbl_name = '_rupa_migrations')
      AND name <> '_rupa_migrations'";

/// Header rows, and header rows for checksum format `?1`.
pub const HEADERS: &str = "
    SELECT count(*), coalesce(sum(checksum_format = ?1), 0)
    FROM main._rupa_migrations
    WHERE kind = 'init'";

/// The journal's largest id, which its `sqlite_sequence` row must match.
pub const MAX_ID: &str = "SELECT coalesce(max(id), 0) FROM main._rupa_migrations";

/// The `sqlite_sequence` rows of the journal.
pub const SEQUENCE: &str = "
    SELECT seq
    FROM main.sqlite_sequence
    WHERE name = '_rupa_migrations'";

/// User objects in `main`, which make a target without a journal unmanaged.
pub const USER_OBJECTS: &str = "
    SELECT count(*)
    FROM main.sqlite_schema
    WHERE substr(lower(name), 1, 7) != 'sqlite_'";

/// Inserts the header row: stream, scheme, checksum format.
pub const INSERT_HEADER: &str = "
    INSERT INTO main._rupa_migrations (kind, schema_version, stream, scheme, checksum_format)
    VALUES ('init', 1, ?1, ?2, ?3)";

/// The stored stream, scheme and owner claim.
pub const HEADER: &str = "
    SELECT stream, scheme, owner
    FROM main._rupa_migrations
    WHERE kind = 'init'";

/// The oldest running nontransactional attempt.
pub const UNRESOLVED: &str = "
    SELECT id
    FROM main._rupa_migrations
    WHERE kind IN ('up', 'down') AND status = 'running' AND policy = 'nontransactional'
    ORDER BY id
    LIMIT 1";

/// The running transactional attempts, which reconciliation records as rolled back.
pub const RECONCILABLE: &str = "
    SELECT id
    FROM main._rupa_migrations
    WHERE kind IN ('up', 'down') AND status = 'running' AND policy = 'transactional'
    ORDER BY id";

/// Records every running transactional attempt as rolled back.
pub const RECONCILE: &str = "
    UPDATE main._rupa_migrations
    SET status = 'rolled_back', finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
    WHERE kind IN ('up', 'down') AND status = 'running' AND policy = 'transactional'";

/// Attempt `?1`.
pub const ATTEMPT: &str = "
    SELECT version, checksum, status
    FROM main._rupa_migrations
    WHERE id = ?1 AND kind IN ('up', 'down')";

/// Journals a running attempt: kind, version, checksum, policy, undone installation.
pub const INSERT_ATTEMPT: &str = "
    INSERT INTO main._rupa_migrations (kind, version, checksum, policy, status, undoes)
    VALUES (?1, ?2, ?3, ?4, 'running', ?5)";

/// Finishes running attempt `?1` with status `?2`, reason `?3` and evidence `?4`.
pub const FINISH: &str = "
    UPDATE main._rupa_migrations
    SET status = ?2, reason = ?3, evidence = ?4,
        finished_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
    WHERE id = ?1 AND kind IN ('up', 'down') AND status = 'running'";

/// Installed history, in installation order: completed installations no
/// completed reversal undoes.
pub const HISTORY: &str = "
    SELECT id, version, checksum, status
    FROM main._rupa_migrations AS installed
    WHERE kind = 'up' AND status IN ('applied', 'recovered')
      AND NOT EXISTS (
          SELECT 1 FROM main._rupa_migrations AS reversal
          WHERE reversal.kind = 'down' AND reversal.undoes = installed.id
            AND reversal.status IN ('applied', 'recovered'))
    ORDER BY id";

/// A fresh owner token.
pub const TOKEN: &str = "SELECT lower(hex(randomblob(16)))";

/// Claims an unowned target for token `?1`.
pub const CLAIM: &str = "
    UPDATE main._rupa_migrations
    SET owner = ?1
    WHERE kind = 'init' AND owner IS NULL";

/// The stored owner claim.
pub const OWNER: &str = "SELECT owner FROM main._rupa_migrations WHERE kind = 'init'";

/// Releases the claim of token `?1`.
pub const UNCLAIM: &str = "
    UPDATE main._rupa_migrations
    SET owner = NULL
    WHERE kind = 'init' AND owner = ?1";

/// Records that an operator released a stale claim: reason `?1`, evidence `?2`.
pub const CLAIM_RELEASED: &str = "
    INSERT INTO main._rupa_migrations (kind, reason, evidence)
    VALUES ('claim_released', ?1, ?2)";

/// `writable_schema`, which must be off.
pub const WRITABLE_SCHEMA: &str = "PRAGMA writable_schema";

/// Attached databases besides `main` and `temp`.
pub const ATTACHED: &str = "
    SELECT count(*)
    FROM pragma_database_list
    WHERE name NOT IN ('main', 'temp')";

/// Objects in the `temp` namespace.
pub const TEMPORARY: &str = "SELECT count(*) FROM temp.sqlite_schema";

/// Opens the read snapshot inspection reads in, without a writer lock.
pub const READ: &str = "BEGIN DEFERRED";

/// The running attempts, oldest first.
pub const RUNNING: &str = "
    SELECT id, version, checksum, policy, kind, undoes
    FROM main._rupa_migrations
    WHERE kind IN ('up', 'down') AND status = 'running'
    ORDER BY id";
