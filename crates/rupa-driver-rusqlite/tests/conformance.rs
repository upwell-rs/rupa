//! The shared conformance suite on a real SQLite (in memory; no Docker).

use rupa_conformance::{Blocking, Harness, SQLITE_ATTACH, SQLITE_DDL};
use rupa_driver_rusqlite::SqliteExecutor;

struct Sqlite(Blocking<SqliteExecutor>);

impl Harness for Sqlite {
    type Exec = Blocking<SqliteExecutor>;

    async fn reset(&mut self) {
        self.0.0.clean().expect("clean");
        self.0
            .0
            .connection()
            .execute_batch(SQLITE_DDL)
            .expect("reset schema");
    }

    fn exec(&mut self) -> &mut Blocking<SqliteExecutor> {
        &mut self.0
    }
}

#[test]
fn conformance() {
    let mut exec = SqliteExecutor::open_in_memory().unwrap();
    exec.connection().execute_batch(SQLITE_ATTACH).unwrap();
    let mut h = Sqlite(Blocking(exec));
    pollster::block_on(rupa_conformance::run(&mut h));
    pollster::block_on(rupa_conformance::run_transactions(&mut h));
}
