//! The conformance suite against a real MySQL 8 (Docker): on one executor,
//! then through the registry's pooled `Database`.

use rupa::core::exec::Blocking;
use rupa::core::tx::TxConnection;
use rupa::{DbConfig, MySqlOptions, Registry};
use rupa_conformance::{Harness, MYSQL_DDL};
use rupa_driver_mysql::{MySqlDriver, MySqlExecutor};
use testcontainers_modules::mysql::Mysql;
use testcontainers_modules::testcontainers::runners::SyncRunner;

struct My(Blocking<MySqlExecutor>);

impl Harness for My {
    type Exec = Blocking<MySqlExecutor>;

    async fn reset(&mut self) {
        self.0.0.clean().expect("clean");
        self.0.0.script(MYSQL_DDL).expect("reset schema");
    }

    fn exec(&mut self) -> &mut Blocking<MySqlExecutor> {
        &mut self.0
    }
}

#[test]
fn conformance() {
    let node = Mysql::default().start().expect("start mysql container");
    let url = format!(
        "mysql://root@127.0.0.1:{}/test",
        node.get_host_port_ipv4(3306).unwrap()
    );
    let opts = mysql::Opts::from_url(&url).unwrap();
    // MySQL restarts once during initialization; retry until it accepts.
    let exec = (0..60)
        .find_map(|_| {
            MySqlExecutor::connect(opts.clone())
                .inspect_err(|_| std::thread::sleep(std::time::Duration::from_millis(500)))
                .ok()
        })
        .expect("connect");
    let mut h = My(Blocking(exec));
    pollster::block_on(rupa_conformance::run(&mut h));
    pollster::block_on(rupa_conformance::run_transactions(&mut h));

    let config = DbConfig::new(&url).max_connections(3).driver(MySqlOptions {
        statement_cache_size: Some(64),
        init: vec!["SET time_zone = '+00:00'".into()],
    });
    let db = pollster::block_on(Registry::new().with(MySqlDriver).connect(&config)).expect("pool");
    pollster::block_on(rupa_conformance::run_database(&db, MYSQL_DDL));
}
