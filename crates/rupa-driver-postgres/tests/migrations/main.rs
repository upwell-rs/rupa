//! Runs the shared migration cases and Postgres-specific migration behaviour
//! against a real Postgres in Docker: one container, reset between cases.

mod postgres_cases;

use std::time::Duration;

use rupa_conformance::migrations::asynchronous::{self, Blocked};
use rupa_conformance::migrations::sync::{self, Harness, Target};
use rupa_driver_postgres::PgExecutor;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::SyncRunner;

/// The `public` schema of a disposable Postgres database.
struct Pg {
    url: String,
}

impl Pg {
    fn client(&self) -> postgres::Client {
        postgres::Client::connect(&self.url, postgres::NoTls).expect("connect")
    }
}

impl Harness for Pg {
    type Session = PgExecutor;

    fn connect(&self) -> PgExecutor {
        PgExecutor::new(self.client()).with_migration_lock_timeout(Duration::from_millis(250))
    }
}

impl Target for Pg {
    fn reset(&mut self) {
        self.execute("DROP SCHEMA IF EXISTS public, tenant CASCADE; CREATE SCHEMA public;");
    }

    fn execute(&self, sql: &str) {
        self.client().batch_execute(sql).expect("setup SQL");
    }

    fn count(&self, sql: &str) -> i64 {
        self.client()
            .query_one(sql, &[])
            .expect("inspection count")
            .get(0)
    }

    fn table_exists(&self, name: &str) -> bool {
        self.client()
            .query_one(
                "SELECT to_regclass('public.' || quote_ident($1)) IS NOT NULL",
                &[&name],
            )
            .expect("schema inspection")
            .get(0)
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
    let node = Postgres::default()
        .start()
        .expect("start postgres container");
    let mut pg = Pg {
        url: format!(
            "postgres://postgres:postgres@{}:{}/postgres",
            node.get_host().expect("container host"),
            node.get_host_port_ipv4(5432).expect("container port")
        ),
    };

    sync::run(&mut pg);

    let mut blocked = Blocked(pg);

    asynchronous::run(&mut blocked);
    postgres_cases::run(&mut blocked.0);
}
