//! Runs the shared migration cases through native async tokio-postgres
//! sessions, plus async-only cancellation behaviour, against a real Postgres
//! in Docker: one container, reset between cases.

mod cancellation;

use std::future::Future;
use std::time::Duration;

use rupa_conformance::migrations::asynchronous::{self, Harness, Target};
use rupa_driver_tokio_postgres::PgExecutor;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use tokio::runtime::Runtime;

/// The `public` schema of a disposable Postgres database, with the runtime
/// that drives its connections.
struct Pg {
    url: String,
    runtime: Runtime,
}

impl Pg {
    async fn client(url: &str) -> tokio_postgres::Client {
        let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
            .await
            .expect("connect");

        tokio::spawn(connection);

        client
    }

    fn inspect<T>(&self, query: impl AsyncFnOnce(tokio_postgres::Client) -> T) -> T {
        self.runtime
            .block_on(async { query(Self::client(&self.url).await).await })
    }
}

impl Harness for Pg {
    type Session = PgExecutor;

    fn connect(&self) -> PgExecutor {
        self.runtime.block_on(async {
            PgExecutor::new(Self::client(&self.url).await)
                .with_migration_lock_timeout(Duration::from_millis(250))
        })
    }

    fn block_on<F: Future>(&self, future: F) -> F::Output {
        self.runtime.block_on(future)
    }
}

impl Target for Pg {
    fn reset(&mut self) {
        self.execute("DROP SCHEMA IF EXISTS public CASCADE; CREATE SCHEMA public;");
    }

    fn execute(&self, sql: &str) {
        self.inspect(async |client| client.batch_execute(sql).await.expect("setup SQL"));
    }

    fn count(&self, sql: &str) -> i64 {
        self.inspect(async |client| {
            client
                .query_one(sql, &[])
                .await
                .expect("inspection count")
                .get(0)
        })
    }

    fn table_exists(&self, name: &str) -> bool {
        self.inspect(async |client| {
            client
                .query_one(
                    "SELECT to_regclass('public.' || quote_ident($1)) IS NOT NULL",
                    &[&name],
                )
                .await
                .expect("schema inspection")
                .get(0)
        })
    }

    fn forbidden_scripts(&self) -> &'static [&'static str] {
        &[
            "CREATE TABLE marker(id INTEGER); SET search_path = pg_catalog;",
            "CREATE TABLE marker(id INTEGER); RESET ALL;",
            "CREATE TABLE marker(id INTEGER); DISCARD ALL;",
            "CREATE TABLE marker(id INTEGER); SELECT pg_advisory_unlock_all();",
            "CREATE TABLE marker(id INTEGER); COPY marker FROM STDIN;",
            "CREATE TABLE marker(id INTEGER); PREPARE TRANSACTION 'escape';",
        ]
    }
}

#[test]
fn migrations() {
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
    let mut pg = Pg { url, runtime };
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        asynchronous::run(&mut pg);
        cancellation::run(&mut pg);
    }));

    pg.runtime.block_on(async move { drop(node) });

    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}
