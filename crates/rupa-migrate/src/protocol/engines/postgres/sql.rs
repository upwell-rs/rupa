//! The PostgreSQL migration protocol's SQL, over the journal table that
//! `sql/postgres.sql` defines. Statements that name the journal take its
//! name qualified with the owned namespace.

/// The journal table.
pub const JOURNAL: &str = "_rupa_migrations";

/// The journal's definition, which names the journal unqualified.
pub const CREATE: &str = include_str!("../../../../sql/postgres.sql");

/// [`CREATE`] with the journal qualified as `journal`.
pub fn create(journal: &str) -> String {
    CREATE.replacen(
        &format!("CREATE TABLE {JOURNAL}"),
        &format!("CREATE TABLE {journal}"),
        1,
    )
}

/// Every reserved relation the journal creates: the table, its primary key
/// and its identity sequence.
pub const RELATIONS: [&str; 3] = [JOURNAL, "_rupa_migrations_pkey", "_rupa_migrations_id_seq"];

/// The journal's columns, as [`SIGNATURE`] reports them.
pub const COLUMNS: &str = "id bigint not null, kind text not null, version text, checksum bytea, policy text, status text, reason text, evidence text, started_at timestamp with time zone not null, finished_at timestamp with time zone, schema_version integer, stream text, scheme text, checksum_format text, owner text, undoes bigint";

/// The column signature of table `$2` in namespace `$1`.
pub const SIGNATURE: &str = "
    SELECT coalesce(string_agg(
        a.attname || ' ' || format_type(a.atttypid, a.atttypmod)
            || CASE WHEN a.attnotnull THEN ' not null' ELSE '' END,
        ', ' ORDER BY a.attnum
    ), '')
    FROM pg_attribute a
    JOIN pg_class c ON c.oid = a.attrelid
    JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname = $1 AND c.relname = $2 AND c.relkind = 'r'
      AND a.attnum > 0 AND NOT a.attisdropped";

/// One row describing the connection: the transaction start, the string
/// setting, the database and the current schema.
pub const DESCRIBE: &str = "
    SELECT transaction_timestamp()::text,
           current_setting('standard_conforming_strings'),
           current_database(),
           current_schema()";

/// The transaction start, read again in a later request: outside a
/// transaction every request runs in its own implicit one, so the two reads
/// agree only inside an application transaction, under either protocol.
pub const TRANSACTION_START: &str = "SELECT transaction_timestamp()::text";

/// Whether this session holds the advisory lock split into `$1`/`$2`.
pub const HOLDS_LOCK: &str = "
    SELECT EXISTS (
        SELECT 1 FROM pg_locks
        WHERE locktype = 'advisory' AND pid = pg_backend_pid() AND granted
          AND objsubid = 1 AND classid::bigint = $1 AND objid::bigint = $2
    )";

/// Reserved relations present in namespace `$1`.
pub const RESERVED_COUNT: &str = "
    SELECT count(*)
    FROM pg_class c
    JOIN pg_namespace n ON n.oid = c.relnamespace
    WHERE n.nspname = $1 AND left(lower(c.relname), 6) = '_rupa_'";

/// User objects in namespace `$1`, which make a target without a journal unmanaged.
pub const USER_OBJECTS: &str = "
    SELECT (SELECT count(*) FROM pg_class c WHERE c.relnamespace = n.oid)
         + (SELECT count(*) FROM pg_proc p WHERE p.pronamespace = n.oid)
         + (SELECT count(*) FROM pg_type t
            WHERE t.typnamespace = n.oid AND t.typtype <> 'c' AND t.typcategory <> 'A')
    FROM pg_namespace n
    WHERE n.nspname = $1";

/// In namespace `$1`: unexpected reserved relations, triggers on the
/// journal, header rows, and header rows for checksum format `$2`.
pub fn integrity(journal: &str) -> String {
    let relations = RELATIONS.map(|name| format!("'{name}'")).join(", ");

    format!(
        "
        SELECT
            (SELECT count(*) FROM pg_class c
             JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE n.nspname = $1 AND left(lower(c.relname), 6) = '_rupa_'
               AND c.relname NOT IN ({relations})),
            (SELECT count(*) FROM pg_trigger t
             JOIN pg_class c ON c.oid = t.tgrelid
             JOIN pg_namespace n ON n.oid = c.relnamespace
             WHERE n.nspname = $1 AND c.relname = '{JOURNAL}' AND NOT t.tgisinternal),
            (SELECT count(*) FROM {journal} WHERE kind = 'init'),
            (SELECT count(*) FROM {journal} WHERE kind = 'init' AND checksum_format = $2)"
    )
}

/// Inserts the header row: stream, scheme, checksum format.
pub fn insert_header(journal: &str) -> String {
    format!(
        "
        INSERT INTO {journal} (kind, schema_version, stream, scheme, checksum_format)
        VALUES ('init', 1, $1, $2, $3)"
    )
}

/// The stored stream and scheme, and the oldest running nontransactional attempt.
pub fn header(journal: &str) -> String {
    format!(
        "
        SELECT stream, scheme,
               (SELECT min(id) FROM {journal}
                WHERE kind IN ('up', 'down') AND status = 'running' AND policy = 'nontransactional')
        FROM {journal}
        WHERE kind = 'init'"
    )
}

/// Installed history, in installation order: completed installations no
/// completed reversal undoes.
pub fn history(journal: &str) -> String {
    format!(
        "
        SELECT id, version, checksum, status
        FROM {journal} AS installed
        WHERE kind = 'up' AND status IN ('applied', 'recovered')
          AND NOT EXISTS (
              SELECT 1 FROM {journal} AS reversal
              WHERE reversal.kind = 'down' AND reversal.undoes = installed.id
                AND reversal.status IN ('applied', 'recovered'))
        ORDER BY id"
    )
}

/// Journals a running attempt of kind `$1`, version `$2`, checksum `$3` and
/// policy `$4`, undoing installation `$5`, returning its id.
pub fn insert_attempt(journal: &str) -> String {
    format!(
        "
        INSERT INTO {journal} (kind, version, checksum, policy, status, undoes)
        VALUES ($1, $2, $3, $4, 'running', $5)
        RETURNING id"
    )
}

/// Finishes running attempt `$1` with status `$2`, reason `$3` and evidence `$4`.
pub fn finish(journal: &str) -> String {
    format!(
        "
        UPDATE {journal}
        SET status = $2, reason = $3, evidence = $4, finished_at = clock_timestamp()
        WHERE id = $1 AND kind IN ('up', 'down') AND status = 'running'"
    )
}

/// Records every running transactional attempt as rolled back, returning their ids.
pub fn reconcile(journal: &str) -> String {
    format!(
        "
        UPDATE {journal}
        SET status = 'rolled_back', finished_at = clock_timestamp()
        WHERE kind IN ('up', 'down') AND status = 'running' AND policy = 'transactional'
        RETURNING id"
    )
}

/// Attempt `$1`, locked for its resolution.
pub fn attempt(journal: &str) -> String {
    format!(
        "
        SELECT version, checksum, status
        FROM {journal}
        WHERE id = $1 AND kind IN ('up', 'down')
        FOR UPDATE"
    )
}

/// Opens the read-only snapshot inspection reads in, which the server
/// refuses to write through.
pub const READ_ONLY: &str = "BEGIN ISOLATION LEVEL REPEATABLE READ READ ONLY";

/// Whether any session of the current database holds the advisory lock
/// split into `$1`/`$2`.
pub const LOCKED: &str = "
    SELECT EXISTS (
        SELECT 1 FROM pg_locks
        WHERE locktype = 'advisory' AND granted
          AND database = (SELECT oid FROM pg_database WHERE datname = current_database())
          AND objsubid = 1 AND classid::bigint = $1 AND objid::bigint = $2
    )";

/// The stored stream and scheme.
pub fn stored(journal: &str) -> String {
    format!(
        "
        SELECT stream, scheme
        FROM {journal}
        WHERE kind = 'init'"
    )
}

/// The running attempts, oldest first.
pub fn running(journal: &str) -> String {
    format!(
        "
        SELECT id, version, checksum, policy, kind, undoes
        FROM {journal}
        WHERE kind IN ('up', 'down') AND status = 'running'
        ORDER BY id"
    )
}
