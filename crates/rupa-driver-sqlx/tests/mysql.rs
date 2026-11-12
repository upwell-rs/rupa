//! The conformance suite against a real MySQL 8 in Docker: on one executor,
//! then through the registry's pooled `Database`.

use rupa::{AsyncTransactional, AsyncTxConnection};
use rupa::{DbConfig, MySqlOptions, Registry};
use rupa_conformance::{Harness, MYSQL_DDL};
use rupa_driver_sqlx::mysql::{MySqlDriver, MySqlExecutor};
use testcontainers_modules::mysql::Mysql;
use testcontainers_modules::testcontainers::runners::AsyncRunner;

struct My(MySqlExecutor);

impl Harness for My {
    type Exec = MySqlExecutor;

    async fn reset(&mut self) {
        self.0.script(MYSQL_DDL).await.expect("reset schema");
    }

    fn exec(&mut self) -> &mut MySqlExecutor {
        &mut self.0
    }
}

#[tokio::test]
async fn conformance() {
    let node = Mysql::default()
        .start()
        .await
        .expect("start mysql container");
    let url = format!(
        "mysql://root@127.0.0.1:{}/test",
        node.get_host_port_ipv4(3306).await.unwrap()
    );
    // MySQL restarts once during initialization; retry until it accepts.
    let mut exec = None;
    for _ in 0..60 {
        match MySqlExecutor::connect(&url).await {
            Ok(e) => {
                exec = Some(e);
                break;
            }
            Err(_) => tokio::time::sleep(std::time::Duration::from_millis(500)).await,
        }
    }

    let mut h = My(exec.expect("connect"));
    rupa_conformance::run(&mut h).await;
    rupa_conformance::run_transactions(&mut h).await;
    dropped_tx_marks_the_connection_dirty(&mut h.0).await;

    let config = DbConfig::new(&url)
        .max_connections(3)
        .driver(rupa::DriverConfig::SqlxMySql(MySqlOptions {
            statement_cache_size: Some(64),
            init: vec!["SET SESSION sql_mode = CONCAT(@@sql_mode, ',STRICT_ALL_TABLES')".into()],
        }));
    let db = Registry::new()
        .with(MySqlDriver::new())
        .connect(&config)
        .await
        .expect("pool");
    rupa_conformance::run_database(&db, MYSQL_DDL).await;
}

async fn dropped_tx_marks_the_connection_dirty(exec: &mut MySqlExecutor) {
    exec.script(MYSQL_DDL).await.unwrap();
    drop(exec.begin().await.unwrap());
    assert!(
        exec.is_dirty(),
        "drop cannot await, so the rollback is owed"
    );
    exec.clean().await.unwrap();
    assert!(!exec.is_dirty());
    // No transaction is open: a savepoint can't be released.
    exec.script("SAVEPOINT probe").await.unwrap();
    assert!(
        exec.script("ROLLBACK TO SAVEPOINT probe").await.is_err(),
        "outside a transaction, MySQL drops savepoints at once"
    );
}
