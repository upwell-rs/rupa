use rupa::core::exec::Executor;
use rupa::migrate::{
    Catalog, Engine, Error, MigrationInfo, RunError, Session, StateError, Target, VersionScheme,
};
use rupa_conformance::migrations::MigrationHarness;
use rupa_conformance::migrations::fixture::body;
use rupa_driver_postgres::PgError;

use super::Pg;

pub(super) fn run(pg: &mut Pg) {
    macro_rules! cases {
        ($($case:ident),* $(,)?) => {$(
            pg.reset();
            eprintln!("postgres migrations {}", stringify!($case));
            $case(pg);
        )*};
    }

    cases!(
        application_transactions_are_rejected,
        nonstandard_strings_are_rejected,
        dollar_bodies_and_escape_strings_run_whole,
        advisory_lock_uses_the_canonical_target_identity,
        schemas_are_independent_targets,
        lost_advisory_lock_poisons_instead_of_committing,
    );
}

fn numbered<M: MigrationInfo>(entries: &[M]) -> Catalog<'_, M> {
    Catalog::new("primary", VersionScheme::Numbered, entries).expect("valid catalog")
}

fn rejected_history<T, E: std::error::Error, M: std::error::Error>(
    result: Result<T, RunError<E, M>>,
) -> bool {
    matches!(
        result,
        Err(RunError::Infrastructure {
            source: Error::History(_),
            ..
        })
    )
}

fn application_transactions_are_rejected(pg: &mut Pg) {
    let entries = [body("1", "CREATE TABLE kept (id INTEGER);")];

    let mut session = pg.connect();
    session
        .client()
        .batch_execute("BEGIN")
        .expect("application transaction");

    assert!(rejected_history(session.apply(&numbered(&entries))));

    session
        .client()
        .batch_execute("ROLLBACK")
        .expect("end application transaction");

    assert!(!pg.table_exists("_rupa_migration_meta"));
}

fn nonstandard_strings_are_rejected(pg: &mut Pg) {
    let entries = [body("1", "CREATE TABLE kept (id INTEGER);")];

    let mut session = pg.connect();
    session
        .client()
        .batch_execute("SET standard_conforming_strings = off")
        .expect("legacy string setting");

    assert!(rejected_history(session.apply(&numbered(&entries))));
    assert!(!pg.table_exists("_rupa_migration_meta"));
}

fn dollar_bodies_and_escape_strings_run_whole(pg: &mut Pg) {
    let entries = [body(
        "1",
        "CREATE FUNCTION add_one(i integer) RETURNS integer AS $$ BEGIN RETURN i + 1; END; $$ LANGUAGE plpgsql; CREATE TABLE quotes (v text); INSERT INTO quotes VALUES (E'it\\'s; fine'), ('semi;colon');",
    )];

    pg.connect()
        .apply(&numbered(&entries))
        .expect("semicolons inside bodies and strings are not boundaries");

    assert_eq!(
        pg.count("SELECT count(*) FROM quotes WHERE v IN ('it''s; fine', 'semi;colon') AND add_one(1) = 2"),
        2
    );
}

fn advisory_lock_uses_the_canonical_target_identity(pg: &mut Pg) {
    let key = Target {
        engine: Engine::Postgres,
        database: "postgres",
        namespace: "public",
    }
    .advisory_key();
    let held = format!(
        "SELECT count(*) FROM pg_locks WHERE locktype = 'advisory' AND granted AND objsubid = 1 AND classid::bigint = {} AND objid::bigint = {}",
        (key >> 32) as u32,
        key as u32
    );

    let mut owner = pg.connect();
    owner
        .acquire("primary", VersionScheme::Numbered)
        .expect("acquire");

    assert_eq!(pg.count(&held), 1, "any driver can derive the held key");

    owner.release().expect("release");

    assert_eq!(pg.count(&held), 0);
}

fn schemas_are_independent_targets(pg: &mut Pg) {
    let entries = [body("1", "CREATE TABLE tenant_rows (id INTEGER);")];

    pg.execute("CREATE SCHEMA tenant");

    let mut public = pg.connect();
    public
        .acquire("primary", VersionScheme::Numbered)
        .expect("acquire public");
    let mut tenant = pg.connect();
    tenant
        .client()
        .batch_execute("SET search_path = tenant")
        .expect("select tenant schema");

    assert_eq!(
        tenant
            .apply(&numbered(&entries))
            .expect("another schema is another target")
            .applied
            .len(),
        1
    );

    public.release().expect("release public");

    assert_eq!(
        pg.count("SELECT count(*) FROM tenant._rupa_migration_history"),
        1
    );
    assert!(!pg.table_exists("tenant_rows"));
}

fn lost_advisory_lock_poisons_instead_of_committing(pg: &mut Pg) {
    let entries = [body(
        "1",
        "CREATE TABLE marker(id INTEGER); DO $$ BEGIN PERFORM pg_advisory_unlock_all(); END $$;",
    )];

    let mut session = pg.connect();

    assert!(
        session.apply(&numbered(&entries)).is_err(),
        "opaque bodies pass the grammar, the lock check refuses completion"
    );
    assert!(matches!(
        Executor::run(&mut session, rupa::raw().sql("SELECT 1").execute::<u64>()),
        Err(PgError::Migration(StateError::Poisoned))
    ));
    assert!(!pg.table_exists("marker"));
    assert_eq!(pg.count("SELECT count(*) FROM _rupa_migration_history"), 0);
    assert_eq!(
        pg.count("SELECT count(*) FROM _rupa_migration_attempts WHERE status = 'running'"),
        1
    );
}
