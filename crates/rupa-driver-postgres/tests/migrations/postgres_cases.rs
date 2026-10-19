use std::time::Duration;

use rupa::core::exec::Executor;
use rupa::migrate::sync::{Apply, Session};
use rupa::migrate::{
    Catalog, Engine, Error, MigrationInfo, RunError, StateError, Target, VersionScheme,
};
use rupa_conformance::migrations::fixture::body;
use rupa_conformance::migrations::sync::{Harness, Target as _};
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
        unbounded_lock_timeouts_are_clamped,
        sync_and_async_drivers_share_one_lock_and_history,
        an_abandoned_pooled_session_is_discarded,
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

fn unbounded_lock_timeouts_are_clamped(pg: &mut Pg) {
    let entries = [body("1", "CREATE TABLE kept (id INTEGER);")];

    let session = pg.connect().with_migration_lock_timeout(Duration::MAX);

    assert_eq!(
        numbered(&entries)
            .apply(session)
            .expect("a wait beyond lock_timeout's maximum still acquires")
            .applied
            .len(),
        1
    );
}

fn sync_and_async_drivers_share_one_lock_and_history(pg: &mut Pg) {
    use rupa::migrate::asynchronous::Session as AsyncSession;

    let first = [body("1", "CREATE TABLE first_rows (id INTEGER);")];
    let both = [
        first[0],
        body("2", "CREATE TABLE second_rows (id INTEGER);"),
    ];
    let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");

    let mut native = runtime.block_on(async {
        let (client, connection) = tokio_postgres::connect(&pg.url, tokio_postgres::NoTls)
            .await
            .expect("async connect");

        tokio::spawn(connection);

        rupa_driver_tokio_postgres::PgExecutor::new(client)
    });
    runtime
        .block_on(AsyncSession::acquire(
            &mut native,
            "primary",
            VersionScheme::Numbered,
        ))
        .expect("async runner acquires");

    assert!(
        matches!(
            numbered(&first).apply(pg.connect()),
            Err(RunError::Infrastructure {
                source: Error::Busy,
                ..
            })
        ),
        "the sync driver derives the same lock key"
    );

    runtime
        .block_on(AsyncSession::release(&mut native))
        .expect("async runner releases");
    numbered(&first)
        .apply(pg.connect())
        .expect("sync runner applies after release");

    let report = runtime
        .block_on(AsyncSession::apply(&mut native, &numbered(&both)))
        .expect("async runner continues the sync history");

    assert_eq!(
        report
            .applied
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["2"]
    );
    assert_eq!(pg.count("SELECT count(*) FROM _rupa_migration_history"), 2);
}

fn an_abandoned_pooled_session_is_discarded(pg: &mut Pg) {
    use rupa::core::pool::Pool;
    use rupa_conformance::migrations::fixture::Interrupted;

    let entries = [Interrupted];
    let catalog = numbered(&entries);
    let config = rupa::DbConfig::new(&pg.url).max_connections(1);
    let pool = rupa_driver_postgres::PgPool::connect(&config).expect("pool");

    let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let mut conn = pool.get().expect("pooled connection");
        let _ = conn.apply(&catalog);
    }));
    let mut replacement = pool.get().expect("replacement connection");

    assert!(interrupted.is_err());
    assert!(
        Executor::run(
            &mut *replacement,
            rupa::raw().sql("SELECT 1").execute::<u64>()
        )
        .is_ok(),
        "the pool replaced the abandoned connection"
    );
}
