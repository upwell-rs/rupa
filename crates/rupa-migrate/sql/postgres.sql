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
-- kind = 'claim_released'  an operator cleared a stale owner claim; only
--                          SQLite runners hold one, in the init row's owner
--
-- Installed history is every 'up' row that is applied or recovered.
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

    CHECK (kind IN ('init', 'up', 'claim_released')),
    CHECK (kind <> 'init' OR (
        schema_version = 1
        AND stream IS NOT NULL
        AND scheme IS NOT NULL
        AND checksum_format IS NOT NULL
    )),
    CHECK (kind <> 'up' OR (
        version IS NOT NULL
        AND length(checksum) = 32
        AND policy IN ('transactional', 'nontransactional')
        AND status IN ('running', 'applied', 'failed', 'rolled_back', 'recovered', 'abandoned')
    ))
)
