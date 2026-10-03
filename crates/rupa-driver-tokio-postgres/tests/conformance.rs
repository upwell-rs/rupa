//! Runs the shared conformance suite against a real Postgres in Docker.

use rupa_conformance::{Harness, POSTGRES_DDL};
use rupa_driver_tokio_postgres::PgExecutor;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;

struct Pg(PgExecutor);

impl Harness for Pg {
    type Exec = PgExecutor;

    async fn reset(&mut self) {
        self.0
            .client()
            .batch_execute(POSTGRES_DDL)
            .await
            .expect("reset schema");
    }

    fn exec(&mut self) -> &mut PgExecutor {
        &mut self.0
    }
}

#[tokio::test]
async fn conformance() {
    let node = Postgres::default()
        .start()
        .await
        .expect("start postgres container");
    let url = format!(
        "postgres://postgres:postgres@{}:{}/postgres",
        node.get_host().await.unwrap(),
        node.get_host_port_ipv4(5432).await.unwrap()
    );
    let (client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
        .await
        .expect("connect");
    tokio::spawn(connection);
    rupa_conformance::run(&mut Pg(PgExecutor::new(client))).await;
}
