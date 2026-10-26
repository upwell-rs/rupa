-- The migration journal Rupa keeps in the target's main database: one
-- table, created on a fresh target and validated on every run. Rows are never
-- deleted, and an attempt's row changes only to record its outcome. Never edit
-- it by hand; use the recovery operations.
--
-- kind = 'init'            the single header row: stream, version scheme and
--                          checksum format the target is managed under.
-- kind = 'up'              one forward attempt per row; its id is the
--                          installation order. status records the outcome:
--     running              journaled before the body ran, still unresolved
--     applied              the body completed and the runner recorded it
--     failed               the body failed and rolled back, or never started
--     rolled_back          interrupted transactional attempt, found later
--                          under exclusion: it provably never committed
--     recovered            an operator asserted its effects are complete
--     abandoned            an operator asserted its effects were undone
-- kind = 'down'            one reversal attempt per row, with the statuses of
--                          'up'; undoes is the id of the installation it
--                          reverses
-- kind = 'claim_released'  an operator cleared a stale owner claim
--
-- owner holds the durable claim of the runner that owns the target, on the
-- init row: SQLite has no session locks to exclude runners between commits.
--
-- Installed history is every 'up' row that is applied or recovered and that no
-- applied or recovered 'down' row undoes. Reapplying an undone artifact adds a
-- new 'up' row, with a new installation rank.
CREATE TABLE _rupa_migrations (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    kind            TEXT    NOT NULL,
    version         TEXT,
    checksum        BLOB,
    policy          TEXT,
    status          TEXT,
    reason          TEXT,
    evidence        TEXT,
    started_at      TEXT    NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    finished_at     TEXT,
    schema_version  INTEGER,
    stream          TEXT,
    scheme          TEXT,
    checksum_format TEXT,
    owner           TEXT,
    undoes          INTEGER,

    CHECK (kind IN ('init', 'up', 'down', 'claim_released')),
    CHECK (kind <> 'init' OR (
        schema_version IS NOT NULL
        AND schema_version = 1
        AND stream IS NOT NULL
        AND scheme IS NOT NULL
        AND checksum_format IS NOT NULL
    )),
    CHECK (kind NOT IN ('up', 'down') OR (
        version IS NOT NULL
        AND checksum IS NOT NULL
        AND typeof(checksum) = 'blob'
        AND length(checksum) = 32
        AND policy IS NOT NULL
        AND policy IN ('transactional', 'nontransactional')
        AND status IS NOT NULL
        AND status IN ('running', 'applied', 'failed', 'rolled_back', 'recovered', 'abandoned')
    )),
    CHECK ((kind = 'down') = (undoes IS NOT NULL))
)
