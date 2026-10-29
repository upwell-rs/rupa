use std::time::Duration;

use rupa::migrate::Policy;
use rupa_conformance::migrations::asynchronous;
use rupa_driver_sqlx::SqlxExecutor;
use testcontainers_modules::mysql::Mysql;
use testcontainers_modules::testcontainers::runners::AsyncRunner;
use tokio::runtime::Runtime;

use super::{Spec, Sqlx, interop, keep};

const SPEC: Spec = Spec {
    reset: "DROP DATABASE IF EXISTS test; CREATE DATABASE test",
    table_exists: "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = DATABASE() AND table_name = ?",
    forbidden: &[
        "CREATE TABLE marker(id INTEGER); SET sql_mode = '';",
        "CREATE TABLE marker(id INTEGER); USE mysql;",
        "CREATE TABLE marker(id INTEGER); SELECT RELEASE_ALL_LOCKS();",
        "CREATE TABLE marker(id INTEGER); LOCK TABLES marker WRITE;",
        "CREATE TABLE marker(id INTEGER) /*! , COMMENT 'x' */;",
        "CREATE TABLE marker(id INTEGER); DELIMITER //",
        "CREATE TABLE marker(id INTEGER); CREATE PROCEDURE p() BEGIN SELECT 1; END",
    ],
    outside: "",
};

#[test]
fn mysql() {
    let runtime = Runtime::new().expect("tokio runtime");
    let (node, url) = runtime.block_on(async {
        let node = Mysql::default()
            .start()
            .await
            .expect("start mysql container");
        let url = format!(
            "mysql://root@127.0.0.1:{}/test",
            node.get_host_port_ipv4(3306).await.expect("container port")
        );

        (node, url)
    });
    let ready = (0..60).any(|_| {
        runtime
            .block_on(SqlxExecutor::<sqlx::MySql>::connect(&url))
            .inspect_err(|_| std::thread::sleep(Duration::from_millis(500)))
            .is_ok()
    });

    assert!(
        ready,
        "MySQL restarts once while initializing, then accepts"
    );

    let mut target = Sqlx::<sqlx::MySql> {
        url,
        runtime,
        spec: SPEC,
        reconnect: keep,
    };
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        asynchronous::run(&mut target);

        eprintln!("migration interop mysql");
        rupa_conformance::migrations::Target::reset(&mut target);
        interop::run(
            || {
                rupa_driver_mysql::MySqlExecutor::connect(
                    mysql::Opts::from_url(&target.url).expect("valid url"),
                )
                .expect("connect")
                .with_migration_lock_timeout(Duration::from_millis(250))
            },
            || {
                target
                    .runtime
                    .block_on(SqlxExecutor::<sqlx::MySql>::connect(&target.url))
                    .expect("connect")
                    .with_migration_lock_timeout(Duration::from_millis(250))
            },
            &target.runtime,
            Policy::NonTransactional,
        );
    }));

    target.runtime.block_on(async move { drop(node) });

    if let Err(panic) = outcome {
        std::panic::resume_unwind(panic);
    }
}
