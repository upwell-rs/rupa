//! `SqlitePool`, owned transactions, and SQLite through the registry.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use std::future::Future;

use rupa::core::exec::{AsyncExecutor, Awaited, Blocking, Engine, Executor, Outcome};
use rupa::core::ir::Statement;
use rupa::core::query::Expect;
use rupa::core::{Dialect, DialectId};
use rupa::prelude::*;
use rupa::{AsyncTransactional, TxConnection};
use rupa::{Database, DbConfig, DriverConfig, Pool, PoolConfig, Registry, SqliteOptions};
use rupa_conformance::{NewNote, Note, SQLITE_ATTACH, SQLITE_DDL};
use rupa_driver_rusqlite::SqliteError;
use rupa_driver_rusqlite::{SqliteDriver, SqliteManager, SqlitePool, SqliteTarget};

#[repository]
trait Notes {
    #[query(insert)]
    fn add(&self, note: &NewNote) -> Result<u64, rupa::DynError>;

    #[query(order_by = id)]
    fn all(&self) -> Result<Vec<Note>, rupa::DynError>;
}

#[repository]
trait AsyncNotes {
    #[query(insert)]
    async fn add(&self, note: &NewNote) -> Result<u64, rupa::DynError>;

    #[query(order_by = id)]
    async fn all(&self) -> Result<Vec<Note>, rupa::DynError>;
}

fn note(title: &str) -> NewNote {
    NewNote {
        title: title.into(),
        pinned: false,
    }
}

/// A native pool with an application policy that rejects transactions.
struct TransactionsDisabled(Blocking<SqlitePool>);

impl Engine for TransactionsDisabled {
    type Completion = Awaited;
}

impl AsyncExecutor for TransactionsDisabled {
    type Dialect = rupa::Sqlite;
    type Error = SqliteError;

    fn dialect(&self) -> &rupa::Sqlite {
        AsyncExecutor::dialect(&self.0)
    }

    fn execute(
        &self,
        statement: &Statement,
        expect: Expect,
    ) -> impl Future<Output = Result<Outcome, SqliteError>> + Send {
        AsyncExecutor::execute(&self.0, statement, expect)
    }
}

impl AsyncTransactional for TransactionsDisabled {
    type Tx<'t> = <Blocking<SqlitePool> as AsyncTransactional>::Tx<'t>;

    async fn begin_with(&self, _: rupa::TxOptions) -> Result<Self::Tx<'_>, SqliteError> {
        Err(SqliteError::Pool(Box::new(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "transactions disabled",
        ))))
    }
}

impl rupa::AsyncPool for TransactionsDisabled {
    type Conn = <Blocking<SqlitePool> as rupa::AsyncPool>::Conn;

    async fn get(&self) -> Result<Self::Conn, SqliteError> {
        rupa::AsyncPool::get(&self.0).await
    }

    fn status(&self) -> rupa::PoolStatus {
        rupa::AsyncPool::status(&self.0)
    }
}

#[test]
fn erased_database_preserves_native_pool_transaction_policy() {
    let file = TempDb::new("repo-native-policy");
    let pool = TransactionsDisabled(Blocking(file.pool(1)));

    pollster::block_on(async {
        let native_error = pool.begin().await.err().expect("native transaction policy");
        let database = Database::new(pool);
        let dynamic_error = database
            .begin()
            .await
            .expect_err("preserve native transaction policy");

        assert!(native_error.to_string().contains("transactions disabled"));
        assert_eq!(dynamic_error.to_string(), native_error.to_string());
    });
}

#[test]
fn a_repository_branches_into_a_pooled_transaction() {
    let file = TempDb::new("repo-open-tx");
    let pool = file.pool(2);
    let notes = NotesRepo::new(&pool);
    let tx = notes.open_tx(None).expect("transaction opens");

    assert_eq!(std::mem::size_of_val(&notes), std::mem::size_of::<&()>());
    assert_eq!(tx.add(&note("committed")).expect("insert note"), 1);
    assert_eq!(tx.all().expect("read own write")[0].title, "committed");
    assert!(
        notes.all().expect("read outside").is_empty(),
        "not visible outside the transaction"
    );
    tx.commit().expect("commit transaction");
    assert_eq!(count(&pool), 1);
}

#[test]
fn a_dropped_repository_transaction_rolls_back() {
    let file = TempDb::new("repo-drop");
    let pool = file.pool(1);
    let notes = NotesRepo::new(&pool);

    notes
        .open_tx(None)
        .expect("transaction opens")
        .add(&note("discarded"))
        .expect("insert note");

    assert_eq!(count(&pool), 0);
}

#[test]
fn a_repository_joins_a_transaction_opened_elsewhere() {
    let file = TempDb::new("repo-join");
    let pool = file.pool(1);
    let notes = NotesRepo::new(&pool);
    let tx = pool.begin().expect("transaction opens");

    notes.tx(&tx).add(&note("joined")).expect("insert note");
    tx.commit().expect("commit transaction");

    assert_eq!(notes.all().expect("read committed").len(), 1);
}

#[test]
fn repositories_run_on_one_executor_and_its_transactions() {
    let file = TempDb::new("repo-executor");
    let pool = file.pool(1);
    let conn = pool.get().expect("connection");
    let notes = NotesRepo::new(&conn);
    let tx = notes.open_tx(None).expect("transaction opens");

    tx.add(&note("one")).expect("insert note");
    tx.commit().expect("commit transaction");

    assert_eq!(notes.all().expect("read committed").len(), 1);
}

#[test]
fn async_repositories_branch_into_transactions_on_a_sync_pool() {
    let file = TempDb::new("repo-async-pool");
    let pool = file.pool(1);
    let notes = AsyncNotesRepo::new(&pool);

    pollster::block_on(async {
        let tx = notes.open_tx(None).await.expect("transaction opens");

        assert_eq!(tx.add(&note("async")).await.expect("insert"), 1);
        assert_eq!(tx.all().await.expect("read own write")[0].title, "async");
        tx.rollback().await.expect("rollback");
        assert!(notes.all().await.expect("read after rollback").is_empty());
    });
}

/// A database file, plus a second one attached as the `conformance` schema;
/// both removed when the test ends.
struct TempDb {
    main: PathBuf,
    schema: PathBuf,
}

impl TempDb {
    fn new(name: &str) -> Self {
        let path = |suffix: &str| {
            std::env::temp_dir().join(format!("rupa-{name}-{}{suffix}.db", std::process::id()))
        };
        let db = Self {
            main: path(""),
            schema: path("-conformance"),
        };
        db.remove();
        db
    }

    fn url(&self) -> String {
        format!("sqlite://{}", self.main.display())
    }

    /// A pool whose connections all see the `conformance` schema, with an
    /// empty `notes` table in it.
    ///
    /// SQLite has one writer at a time, so concurrent writers wait for the
    /// lock. Both databases use WAL (`journal_mode` on the manager only
    /// covers the main one, but the tables live in the attached one), and the
    /// busy timeout is generous: on a slow CI disk, the 5s default was
    /// sometimes not enough for 8 threads, and the inserts failed with
    /// "database is locked".
    fn pool(&self, size: u32) -> SqlitePool {
        let attach = format!("ATTACH DATABASE '{}' AS conformance", self.schema.display());
        let manager = SqliteManager::new(SqliteTarget::parse(&self.url()).unwrap())
            .journal_mode("wal")
            .busy_timeout(Some(Duration::from_secs(60)))
            .on_connect(move |c| {
                c.execute_batch(&attach)?;
                c.pragma_update_and_check(Some("conformance"), "journal_mode", "wal", |_| Ok(()))
            });
        let pool = SqlitePool::new(manager, &PoolConfig::default().max_size(size)).unwrap();
        pool.get()
            .unwrap()
            .connection()
            .execute_batch(SQLITE_DDL)
            .unwrap();
        pool
    }

    fn remove(&self) {
        for path in [&self.main, &self.schema] {
            for suffix in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{suffix}", path.display()));
            }
        }
    }
}

impl Drop for TempDb {
    fn drop(&mut self) {
        self.remove();
    }
}

fn add(conn: &impl Executor, title: &str) {
    let note = NewNote {
        title: title.into(),
        pinned: false,
    };
    assert_eq!(
        insert::<Note>()
            .value(&note)
            .affected()
            .execute(conn)
            .unwrap(),
        1
    );
}

fn count(pool: &SqlitePool) -> usize {
    select::<Note>().all().execute(pool).unwrap().len()
}

#[test]
fn conformance_through_a_database() {
    // An in-memory database: the pool keeps its one connection.
    let manager =
        SqliteManager::new(SqliteTarget::Memory).on_connect(|c| c.execute_batch(SQLITE_ATTACH));
    let pool = SqlitePool::new(manager, &PoolConfig::default()).unwrap();
    assert_eq!(pool.status().max_size, 1, "memory databases are not shared");
    let db = Database::new(Blocking(pool));
    pollster::block_on(rupa_conformance::run_database(&db, SQLITE_DDL));
}

#[test]
fn owned_transactions_hold_their_connection() {
    let file = TempDb::new("owned");
    let pool = file.pool(2);

    let tx = pool.begin().unwrap();
    add(&tx, "a");
    assert_eq!(pool.status().in_use(), 1);
    tx.commit().unwrap();
    assert_eq!(
        pool.status().in_use(),
        0,
        "committing returns the connection"
    );

    {
        let tx = pool.begin().unwrap();
        add(&tx, "gone");
    } // dropped: rolled back, then returned
    assert_eq!(count(&pool), 1);
    assert!(!pool.get().unwrap().is_dirty());
}

#[test]
fn pooled_connections_serve_threads() {
    let file = TempDb::new("threads");
    let pool = file.pool(4);
    let shared = Arc::new(pool.clone());
    std::thread::scope(|s| {
        for t in 0..8 {
            let pool = shared.clone();
            s.spawn(move || {
                for i in 0..10 {
                    add(&*pool, &format!("{t}-{i}"));
                }
            });
        }
    });
    assert_eq!(count(&pool), 80);
    assert!(pool.status().size <= 4);
    let wal = PathBuf::from(format!("{}-wal", file.schema.display()));
    assert!(wal.exists(), "the attached schema is in WAL mode too");
}

#[test]
fn registry_builds_a_pool_from_config() {
    let file = TempDb::new("registry");
    let config: DbConfig = serde_json::from_value(serde_json::json!({
        "url": file.url(),
        "busy_timeout": "250ms",
        "journal_mode": "wal",
        "pool": { "max_size": 3 },
    }))
    .unwrap();
    assert_eq!(
        config.driver,
        DriverConfig::Sqlite(SqliteOptions {
            busy_timeout: Some(Duration::from_millis(250)),
            foreign_keys: true,
            journal_mode: Some("wal".into()),
            statement_cache_size: None,
        })
    );
    let db = pollster::block_on(Registry::new().with(SqliteDriver).connect(&config)).unwrap();
    assert_eq!(db.status().max_size, 3);
    assert_eq!(db.dialect().id(), DialectId::Sqlite);

    pollster::block_on(
        raw()
            .sql("CREATE TABLE t (x INTEGER)")
            .execute::<u64>()
            .execute(&db),
    )
    .unwrap();
    let wal = PathBuf::from(format!("{}-wal", file.main.display()));
    assert!(wal.exists(), "journal_mode = wal was applied");
}

#[test]
fn bad_configs_fail_at_connect() {
    let registry = Registry::new().with(SqliteDriver);
    let connect = |c: DbConfig| {
        pollster::block_on(registry.connect(&c))
            .unwrap_err()
            .to_string()
    };
    assert!(connect(DbConfig::new("sqlite://")).contains("missing database path"));
    assert!(
        connect(DbConfig::new("postgres://x/y"))
            .contains("no registered driver accepts `postgres:`")
    );
    assert!(
        connect(DbConfig::new("sqlite://x.db").driver(DriverConfig::Memory))
            .contains("no driver named `memory`")
    );
    let missing = std::env::temp_dir().join("rupa-no-such-dir/x/y.db");
    assert!(
        connect(DbConfig::new(format!("sqlite://{}", missing.display())))
            .contains("could not connect")
    );
}

#[test]
fn repeated_queries_hit_the_render_cache() {
    let file = TempDb::new("render-cache");
    let pool = file.pool(1);
    add(&pool, "first");
    // The process-wide cache is shared with other tests: count our own hits.
    let before = rupa::sql::render_cache().stats().hits;
    for _ in 0..5 {
        count(&pool);
    }

    assert!(rupa::sql::render_cache().stats().hits >= before + 4);
}

#[test]
fn statement_cache_size_is_configurable() {
    let file = TempDb::new("stmt-cache");
    let config: DbConfig = serde_json::from_value(serde_json::json!({
        "url": file.url(),
        "statement_cache_size": 0,
    }))
    .unwrap();
    let DriverConfig::Sqlite(options) = &config.driver else {
        panic!("{config:?}")
    };
    assert_eq!(options.statement_cache_size, Some(0));
    // Still works with prepared statements turned off.
    let pool = SqlitePool::connect(&config).unwrap();
    let mut conn = pool.get().unwrap();
    conn.connection().execute_batch(NOTES_DDL_MAIN).unwrap();
    assert_eq!(
        raw()
            .sql("SELECT 1 FROM notes")
            .rows::<Vec<Note>>()
            .execute(&conn)
            .unwrap(),
        []
    );
}

const NOTES_DDL_MAIN: &str = "CREATE TABLE notes (id INTEGER PRIMARY KEY, title TEXT NOT NULL, body TEXT, pinned INTEGER NOT NULL)";

#[test]
fn connect_one_opens_a_connection_outside_the_pool() {
    let config = DbConfig::new("sqlite::memory:");

    let mut one =
        <SqlitePool as rupa::SyncConnect>::connect_one(&config).expect("a single connection opens");

    one.connection()
        .execute_batch(NOTES_DDL_MAIN)
        .expect("the schema applies");

    let notes = raw()
        .sql("SELECT * FROM notes")
        .rows::<Vec<Note>>()
        .execute(&one)
        .expect("the query runs on the single connection");

    assert_eq!(notes, []);
}
