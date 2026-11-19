use std::time::Duration;

use rupa::AsyncPool;
use rupa::DbConfig;
use rupa::migrate::asynchronous::{Apply, Session};
use rupa::migrate::{Catalog, Error, MigrationInfo, RunError, StateError, VersionScheme};
use rupa_conformance::migrations::asynchronous::{Harness, Target};
use rupa_conformance::migrations::fixture::body;
use rupa_driver_tokio_postgres::{PgError, PgPool};

use super::Pg;

/// Long enough to still be inside the body when the caller gives up.
const SLOW: &str = "CREATE TABLE lost(id INTEGER); SELECT pg_sleep(1);";
const GIVE_UP: Duration = Duration::from_millis(300);

pub(super) fn run(pg: &mut Pg) {
    macro_rules! cases {
        ($($case:ident),* $(,)?) => {$(
            pg.reset();
            eprintln!("postgres async migrations {}", stringify!($case));
            $case(pg);
        )*};
    }

    cases!(
        a_dropped_apply_closes_the_connection_and_its_attempt_is_reconciled,
        a_dropped_apply_poisons_a_retained_session,
        an_abandoned_pooled_session_is_discarded,
    );
}

fn numbered<M: MigrationInfo>(entries: &[M]) -> Catalog<'_, M> {
    Catalog::new("primary", VersionScheme::Numbered, entries).expect("valid catalog")
}

fn a_dropped_apply_closes_the_connection_and_its_attempt_is_reconciled(pg: &mut Pg) {
    let entries = [body("1", SLOW)];
    let catalog = numbered(&entries);

    let session = pg.connect();
    let abandoned =
        pg.block_on(async { tokio::time::timeout(GIVE_UP, catalog.apply(session)).await });
    let competitor = pg.block_on(catalog.apply(pg.connect()));

    assert!(abandoned.is_err(), "the caller gave up mid-body");
    assert!(!pg.table_exists("lost"));

    match competitor {
        Err(RunError::Infrastructure {
            source: Error::Busy,
            ..
        }) => assert_eq!(
            pg.count(
                "SELECT count(*) FROM _rupa_migrations WHERE kind = 'up' AND status = 'running'"
            ),
            1,
            "the server still runs the abandoned body, which holds the lock"
        ),
        Err(RunError::Infrastructure {
            source: Error::Reconciled(attempts),
            ..
        }) => {
            assert_eq!(attempts.len(), 1);
            assert_eq!(
                pg.count(
                    "SELECT count(*) FROM _rupa_migrations WHERE kind = 'up' AND status = 'rolled_back'"
                ),
                1,
                "once the connection closed, the attempt provably rolled back"
            );
        }
        other => panic!("an abandoned run is never replayed in the same apply: {other:?}"),
    }
}

fn a_dropped_apply_poisons_a_retained_session(pg: &mut Pg) {
    let entries = [body("1", SLOW)];
    let catalog = numbered(&entries);

    let mut session = pg.connect();
    let abandoned =
        pg.block_on(async { tokio::time::timeout(GIVE_UP, session.apply(&catalog)).await });
    let reused = pg.block_on(
        rupa::raw()
            .sql("SELECT 1")
            .execute::<u64>()
            .execute(&session),
    );

    assert!(abandoned.is_err(), "the caller gave up mid-body");
    assert!(
        matches!(reused, Err(PgError::Migration(StateError::Poisoned))),
        "a retained session refuses use after an abandoned run"
    );
    assert!(
        matches!(
            pg.block_on(session.acquire("primary", VersionScheme::Numbered)),
            Err(Error::History(_))
        ),
        "it cannot start another run either"
    );
}

fn an_abandoned_pooled_session_is_discarded(pg: &mut Pg) {
    let entries = [body("1", SLOW)];
    let catalog = numbered(&entries);
    let config = DbConfig::new(&pg.url).max_connections(1);

    let pool = pg.block_on(PgPool::connect(&config)).expect("pool");
    let abandoned = pg.block_on(async {
        let mut conn = pool.get().await.expect("pooled connection");

        tokio::time::timeout(GIVE_UP, Session::apply(&mut *conn, &catalog)).await
    });
    let reused = pg.block_on(async {
        let conn = pool.get().await.expect("replacement connection");

        rupa::raw()
            .sql("SELECT 1")
            .execute::<u64>()
            .execute(&conn)
            .await
    });

    assert!(abandoned.is_err(), "the caller gave up mid-body");
    assert!(
        reused.is_ok(),
        "the pool replaced the abandoned connection: {reused:?}"
    );
}
