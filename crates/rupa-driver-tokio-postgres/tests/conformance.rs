//! Runs the shared conformance suite against a real Postgres in Docker.

use rupa_conformance::{Harness, POSTGRES_DDL};
use rupa_driver_tokio_postgres::PgExecutor;
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::AsyncRunner;

struct Pg(PgExecutor);

impl Harness for Pg {
    type Exec = PgExecutor;

    async fn reset(&mut self) {
        self.0.clean().await.expect("clean");
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
    let mut h = Pg(PgExecutor::new(client));
    rupa_conformance::run(&mut h).await;
    rupa_conformance::run_transactions(&mut h).await;
    dropped_tx_marks_the_connection_dirty(&mut h.0).await;
}

/// The async-drop strategy, observed directly: a dropped `PgTx` leaves the
/// executor dirty until the rollback is sent.
async fn dropped_tx_marks_the_connection_dirty(exec: &mut PgExecutor) {
    use rupa::core::tx::AsyncTransactional;

    exec.clean().await.unwrap();
    exec.client().batch_execute(POSTGRES_DDL).await.unwrap();
    assert!(!exec.is_dirty());
    drop(exec.begin().await.unwrap());
    assert!(
        exec.is_dirty(),
        "drop cannot await, so the rollback is owed"
    );
    exec.clean().await.unwrap();
    assert!(!exec.is_dirty());
    // The server agrees: no transaction is open, so SAVEPOINT is refused.
    let outside = exec.client().batch_execute("SAVEPOINT probe").await;
    assert!(outside.is_err(), "the dropped transaction must be closed");
}
