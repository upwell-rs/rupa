//! The shared async migration cases on every sqlx backend, and interop with
//! the native driver of the same engine: both drivers implement one engine
//! protocol, so they exclude each other and read the same history. Postgres
//! and MySQL run in Docker, one container per engine; SQLite uses files.

mod interop;
mod mysql;
mod postgres;
mod sqlite;

use std::future::Future;
use std::time::Duration;

use rupa::core::{SqlType, Value};
use rupa::migrate::asynchronous::Session;
use rupa_conformance::migrations::asynchronous::{Harness, Target};
use rupa_driver_sqlx::{Backend, SqlxExecutor};
use rupa_migrate::protocol::asynchronous::Io;
use tokio::runtime::Runtime;

/// What differs per engine in the harness.
struct Spec {
    /// The owned-namespace reset, as one script.
    reset: &'static str,
    /// Whether table `$1`/`?1` exists, as one integer.
    table_exists: &'static str,
    /// Scripts the engine must refuse beyond the shared ones.
    forbidden: &'static [&'static str],
    /// Statements refused inside a transaction.
    outside: &'static str,
}

/// A sqlx migration target: independent connections to `url`, driven on
/// `runtime`.
struct Sqlx<DB: Backend> {
    url: String,
    runtime: Runtime,
    spec: Spec,
    reconnect: fn(&mut Sqlx<DB>),
}

impl<DB: Backend> Sqlx<DB>
where
    SqlxExecutor<DB>: Io<Error = rupa_driver_sqlx::SqlxError>,
{
    fn inspector(&self) -> SqlxExecutor<DB> {
        self.runtime
            .block_on(SqlxExecutor::<DB>::connect(&self.url))
            .expect("inspection connection")
    }

    fn integer(&self, sql: &str, params: &[Value]) -> i64 {
        let mut inspector = self.inspector();
        let rows = self
            .runtime
            .block_on(Io::query(&mut inspector, sql, params, &[SqlType::I64]))
            .expect("inspection query");

        match rows.as_slice() {
            [row] => match row[0] {
                Value::I64(value) => value,
                ref other => panic!("{sql} returned {other:?}"),
            },
            _ => panic!("{sql} did not return one row"),
        }
    }
}

impl<DB: Backend> Target for Sqlx<DB>
where
    SqlxExecutor<DB>: Io<Error = rupa_driver_sqlx::SqlxError>,
{
    fn reset(&mut self) {
        (self.reconnect)(self);

        if !self.spec.reset.is_empty() {
            self.execute(self.spec.reset);
        }
    }

    fn execute(&self, sql: &str) {
        let inspector = self.inspector();

        self.runtime
            .block_on(inspector.script(sql))
            .expect("setup SQL");
    }

    fn count(&self, sql: &str) -> i64 {
        self.integer(sql, &[])
    }

    fn table_exists(&self, name: &str) -> bool {
        self.integer(self.spec.table_exists, &[Value::Text(name.to_owned())]) != 0
    }

    fn forbidden_scripts(&self) -> &'static [&'static str] {
        self.spec.forbidden
    }

    fn outside_transaction_script(&self) -> &'static str {
        self.spec.outside
    }
}

impl<DB: Backend> Harness for Sqlx<DB>
where
    SqlxExecutor<DB>: Session + Io<Error = rupa_driver_sqlx::SqlxError>,
{
    type Session = SqlxExecutor<DB>;

    fn connect(&self) -> SqlxExecutor<DB> {
        self.inspector()
            .with_migration_lock_timeout(Duration::from_millis(250))
    }

    fn block_on<F: Future>(&self, future: F) -> F::Output {
        self.runtime.block_on(future)
    }
}

/// Keeps the target as it is between cases; the reset script empties it.
fn keep<DB: Backend>(_: &mut Sqlx<DB>) {}
