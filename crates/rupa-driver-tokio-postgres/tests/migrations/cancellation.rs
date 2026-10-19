use std::time::Duration;

use rupa::core::exec::AsyncExecutor;
use rupa::migrate::asynchronous::{Apply, Session};
use rupa::migrate::{Catalog, Error, MigrationInfo, RunError, StateError, VersionScheme};
use rupa_conformance::migrations::asynchronous::{Harness, Target};
use rupa_conformance::migrations::fixture::body;
use rupa_driver_tokio_postgres::PgError;

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
        a_dropped_apply_closes_the_connection_and_leaves_an_unresolved_attempt,
        a_dropped_apply_poisons_a_retained_session,
    );
}

fn numbered<M: MigrationInfo>(entries: &[M]) -> Catalog<'_, M> {
    Catalog::new("primary", VersionScheme::Numbered, entries).expect("valid catalog")
}

fn a_dropped_apply_closes_the_connection_and_leaves_an_unresolved_attempt(pg: &mut Pg) {
    let entries = [body("1", SLOW)];
    let catalog = numbered(&entries);

    let session = pg.connect();
    let abandoned =
        pg.block_on(async { tokio::time::timeout(GIVE_UP, catalog.apply(session)).await });
    let competitor = pg.block_on(catalog.apply(pg.connect()));

    assert!(abandoned.is_err(), "the caller gave up mid-body");
    assert!(
        matches!(
            competitor,
            Err(RunError::Infrastructure {
                source: Error::Busy | Error::Uncertain(_),
                ..
            })
        ),
        "an abandoned run blocks blind replay"
    );
    assert!(!pg.table_exists("lost"));
    assert_eq!(
        pg.count("SELECT count(*) FROM _rupa_migration_attempts WHERE status = 'running'"),
        1
    );
}

fn a_dropped_apply_poisons_a_retained_session(pg: &mut Pg) {
    let entries = [body("1", SLOW)];
    let catalog = numbered(&entries);

    let mut session = pg.connect();
    let abandoned =
        pg.block_on(async { tokio::time::timeout(GIVE_UP, session.apply(&catalog)).await });
    let reused = pg.block_on(session.run(rupa::raw().sql("SELECT 1").execute::<u64>()));

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
