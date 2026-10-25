use std::time::Duration;

use rupa::migrate::Policy;
use rupa_conformance::migrations::asynchronous;
use rupa_driver_sqlx::SqlxExecutor;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use tokio::runtime::Runtime;

use super::{Spec, Sqlx, interop, keep};

const SPEC: Spec = Spec {
    reset: "DROP SCHEMA IF EXISTS public CASCADE; CREATE SCHEMA public;",
    table_exists: "SELECT CASE WHEN to_regclass('public.' || quote_ident($1)) IS NULL THEN 0 ELSE 1 END::int8",
    forbidden: &[
        "CREATE TABLE marker(id INTEGER); SET search_path = pg_catalog;",
        "CREATE TABLE marker(id INTEGER); RESET ALL;",
        "CREATE TABLE marker(id INTEGER); DISCARD ALL;",
        "CREATE TABLE marker(id INTEGER); SELECT pg_advisory_unlock_all();",
        "CREATE TABLE marker(id INTEGER); COPY marker FROM STDIN;",
        "CREATE TABLE marker(id INTEGER); PREPARE TRANSACTION 'escape';",
    ],
    outside: "CREATE INDEX CONCURRENTLY notes_title ON notes (title); CREATE INDEX CONCURRENTLY notes_id ON notes (id);",
};

#[test]
fn postgres() {
    let runtime = Runtime::new().expect("tokio runtime");
    let (node, url) = runtime.block_on(async {
        let node = Postgres::default()
            .start()
            .await
            .expect("start postgres container");
        let url = format!(
            "postgres://postgres:postgres@{}:{}/postgres",
            node.get_host().await.expect("container host"),
            node.get_host_port_ipv4(5432).await.expect("container port")
        );

        (node, url)
    });
    let mut target = Sqlx::<sqlx::Postgres> {
        url,
        runtime,
        spec: SPEC,
        reconnect: keep,
    };
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        asynchronous::run(&mut target);

        eprintln!("migration interop postgres");
        rupa_conformance::migrations::Target::reset(&mut target);
        interop::run(
            || {
                let client =
                    postgres::Client::connect(&target.url, postgres::NoTls).expect("connect");

                rupa_driver_postgres::PgExecutor::new(client)
                    .with_migration_lock_timeout(Duration::from_millis(250))
            },
            || {
                target
                    .runtime
                    .block_on(SqlxExecutor::<sqlx::Postgres>::connect(&target.url))
                    .expect("connect")
                    .with_migration_lock_timeout(Duration::from_millis(250))
            },
            &target.runtime,
            Policy::TransactionalRequired,
        );
    }));

    target.runtime.block_on(async move { drop(node) });

    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}
