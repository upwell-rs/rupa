-- The migration journal Rupa keeps in the target's current schema: one
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
-- kind = 'snapshot'        one attempt of a snapshot on a fresh target, with
--                          the statuses of 'up'; its id is its installation
--                          rank, and completing it records its coverage
-- kind = 'adopt'           an operator adopted an unmanaged target through
--                          version, with a reason and evidence
-- kind = 'covered'         one incremental a completed snapshot or an
--                          adoption covers: its version and checksum, and
--                          covered_by, the id of that snapshot or adoption
-- kind = 'claim_released'  an operator cleared a stale owner claim; only
--                          SQLite runners hold one, in the init row's owner
--
-- Installed history is every 'up' row that is applied or recovered and that no
-- applied or recovered 'down' row undoes. Reapplying an undone artifact adds a
-- new 'up' row, with a new installation rank. Covered incrementals are not
-- installations: they never ran, and nothing undoes them.
CREATE TABLE _rupa_migrations (
    id              bigint      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    kind            text        NOT NULL,
    version         text,
    checksum        bytea,
    policy          text,
    status          text,
    reason          text,
    evidence        text,
    started_at      timestamptz NOT NULL DEFAULT clock_timestamp(),
    finished_at     timestamptz,
    schema_version  integer,
    stream          text,
    scheme          text,
    checksum_format text,
    owner           text,
    undoes          bigint,
    covered_by      bigint,

    CHECK (kind IN ('init', 'up', 'down', 'snapshot', 'adopt', 'covered', 'claim_released')),
    CHECK (kind <> 'init' OR (
        schema_version IS NOT NULL
        AND schema_version = 1
        AND stream IS NOT NULL
        AND scheme IS NOT NULL
        AND checksum_format IS NOT NULL
    )),
    CHECK (kind NOT IN ('up', 'down', 'snapshot') OR (
        version IS NOT NULL
        AND checksum IS NOT NULL
        AND length(checksum) = 32
        AND policy IS NOT NULL
        AND policy IN ('transactional', 'nontransactional')
        AND status IS NOT NULL
        AND status IN ('running', 'applied', 'failed', 'rolled_back', 'recovered', 'abandoned')
    )),
    CHECK (kind <> 'adopt' OR (version IS NOT NULL AND reason IS NOT NULL)),
    CHECK (kind <> 'covered' OR (
        version IS NOT NULL
        AND checksum IS NOT NULL
        AND length(checksum) = 32
    )),
    CHECK ((kind = 'down') = (undoes IS NOT NULL)),
    CHECK ((kind = 'covered') = (covered_by IS NOT NULL))
)
