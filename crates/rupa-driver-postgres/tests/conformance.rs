//! Runs the shared conformance suite against a real Postgres in Docker.

use rupa_conformance::{Blocking, Harness, POSTGRES_DDL};
use rupa_driver_postgres::PgExecutor;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::SyncRunner;

struct Pg(Blocking<PgExecutor>);

impl Harness for Pg {
    type Exec = Blocking<PgExecutor>;

    async fn reset(&mut self) {
        self.0.0.clean().expect("clean");
        self.0
            .0
            .client()
            .batch_execute(POSTGRES_DDL)
            .expect("reset schema");
    }

    fn exec(&mut self) -> &mut Blocking<PgExecutor> {
        &mut self.0
    }
}

#[test]
fn conformance() {
    let node = Postgres::default()
        .start()
        .expect("start postgres container");
    let url = format!(
        "postgres://postgres:postgres@{}:{}/postgres",
        node.get_host().unwrap(),
        node.get_host_port_ipv4(5432).unwrap()
    );
    let client = postgres::Client::connect(&url, postgres::NoTls).expect("connect");
    let mut h = Pg(Blocking(PgExecutor::new(client)));
    pollster::block_on(rupa_conformance::run(&mut h));
    pollster::block_on(rupa_conformance::run_transactions(&mut h));
}
