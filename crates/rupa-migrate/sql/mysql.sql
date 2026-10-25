-- The migration journal Rupa keeps in the target's selected database: one
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
    id              BIGINT       AUTO_INCREMENT PRIMARY KEY,
    kind            VARCHAR(16)  NOT NULL,
    version         VARCHAR(255),
    checksum        BINARY(32),
    policy          VARCHAR(16),
    status          VARCHAR(16),
    reason          TEXT,
    evidence        TEXT,
    started_at      DATETIME(6)  NOT NULL DEFAULT CURRENT_TIMESTAMP(6),
    finished_at     DATETIME(6),
    schema_version  INT,
    stream          VARCHAR(255),
    scheme          VARCHAR(64),
    checksum_format VARCHAR(64),
    owner           VARCHAR(64),

    CHECK (kind IN ('init', 'up', 'claim_released')),
    CHECK (kind <> 'init' OR (
        schema_version = 1
        AND stream IS NOT NULL
        AND scheme IS NOT NULL
        AND checksum_format IS NOT NULL
    )),
    CHECK (kind <> 'up' OR (
        version IS NOT NULL
        AND checksum IS NOT NULL
        AND policy IN ('transactional', 'nontransactional')
        AND status IN ('running', 'applied', 'failed', 'rolled_back', 'recovered', 'abandoned')
    ))
) ENGINE = InnoDB
