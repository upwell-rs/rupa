//! Runs the shared migration cases and MySQL-specific migration behaviour
//! against a real MySQL 8 in Docker: one container, reset between cases.

mod mysql_cases;

use std::time::Duration;

use mysql::prelude::Queryable;
use rupa_conformance::migrations::asynchronous::{self, Blocked};
use rupa_conformance::migrations::sync::{self, Harness, Target};
use rupa_driver_mysql::MySqlExecutor;
use testcontainers_modules::mysql::Mysql;
use testcontainers_modules::testcontainers::runners::SyncRunner;

/// The `test` database of a disposable MySQL server.
struct My {
    opts: mysql::Opts,
}

impl My {
    fn conn(&self) -> mysql::Conn {
        mysql::Conn::new(self.opts.clone()).expect("connect")
    }
}

impl Harness for My {
    type Session = MySqlExecutor;

    fn connect(&self) -> MySqlExecutor {
        MySqlExecutor::connect(self.opts.clone())
            .expect("connect")
            .with_migration_lock_timeout(Duration::from_millis(250))
    }
}

impl Target for My {
    fn reset(&mut self) {
        self.execute("DROP DATABASE IF EXISTS test; CREATE DATABASE test");
    }

    fn execute(&self, sql: &str) {
        MySqlExecutor::new(self.conn())
            .script(sql)
            .expect("setup SQL");
    }

    fn count(&self, sql: &str) -> i64 {
        self.conn()
            .query_first(sql)
            .expect("inspection count")
            .expect("one row")
    }

    fn table_exists(&self, name: &str) -> bool {
        self.conn()
            .exec_first::<i64, _, _>(
                "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema = DATABASE() AND table_name = ?",
                (name,),
            )
            .expect("schema inspection")
            == Some(1)
    }

    fn forbidden_scripts(&self) -> &'static [&'static str] {
        &[
            "CREATE TABLE marker(id INTEGER); SET sql_mode = '';",
            "CREATE TABLE marker(id INTEGER); USE mysql;",
            "CREATE TABLE marker(id INTEGER); SELECT RELEASE_ALL_LOCKS();",
            "CREATE TABLE marker(id INTEGER); LOCK TABLES marker WRITE;",
            "CREATE TABLE marker(id INTEGER) /*! , COMMENT 'x' */;",
            "CREATE TABLE marker(id INTEGER); DELIMITER //",
            "CREATE TABLE marker(id INTEGER); CREATE PROCEDURE p() BEGIN SELECT 1; END",
        ]
    }
}

#[test]
fn migrations() {
    let node = Mysql::default().start().expect("start mysql container");
    let url = format!(
        "mysql://root@127.0.0.1:{}/test",
        node.get_host_port_ipv4(3306).expect("container port")
    );
    let opts = mysql::Opts::from_url(&url).expect("valid url");
    let mut my = My { opts };

    let ready = (0..60).any(|_| {
        mysql::Conn::new(my.opts.clone())
            .inspect_err(|_| std::thread::sleep(Duration::from_millis(500)))
            .is_ok()
    });

    assert!(
        ready,
        "MySQL restarts once while initializing, then accepts"
    );

    sync::run(&mut my);

    let mut blocked = Blocked(my);

    asynchronous::run(&mut blocked);
    mysql_cases::run(&mut blocked.0);
}
