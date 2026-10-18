//! The suite through a type-erased [`Database`]: every driver's pool,
//! boxed executors and boxed (owned) transactions.
//!
//! [`run_database`] runs the executor and transaction suites on one checked-out
//! connection ([`BoxAsyncExecutor`]), then the cases below, which use the
//! pool itself: owned transactions, repositories over the pool, and
//! connection reuse after a dropped transaction.

use std::sync::Arc;

use rupa::core::error::TxError;
use rupa::core::exec::AsyncExecutor;
use rupa::core::tx::{AsyncTransaction, AsyncTransactional};
use rupa::core::{Capability, Dialect};
use rupa::prelude::*;
use rupa::{BoxAsyncExecutor, Database, DynError, SecurityContext, TxOptions};

use crate::{Harness, NewNote, Note};

/// Runs DDL scripts statement by statement through a boxed executor.
pub async fn run_script<E: AsyncExecutor>(e: &mut E, script: &'static str) {
    for statement in script.split(';').map(str::trim).filter(|s| !s.is_empty()) {
        e.run(raw().sql(statement).execute::<u64>())
            .await
            .unwrap_or_else(|err| panic!("DDL `{statement}` failed: {err}"));
    }
}

/// A [`Harness`] on one connection checked out of a [`Database`].
pub struct DatabaseHarness {
    conn: BoxAsyncExecutor,
    ddl: &'static str,
}

impl DatabaseHarness {
    /// Checks a connection out of `db`; `ddl` resets the schema.
    pub async fn new(db: &Database, ddl: &'static str) -> Self {
        Self {
            conn: db.get().await.expect("check out a connection"),
            ddl,
        }
    }
}

impl Harness for DatabaseHarness {
    type Exec = BoxAsyncExecutor;

    async fn reset(&mut self) {
        run_script(&mut self.conn, self.ddl).await;
    }

    fn exec(&mut self) -> &mut BoxAsyncExecutor {
        &mut self.conn
    }
}

/// A dyn-compatible repository over the pool (`Arc<dyn Notes>`), as a DI
/// container would hold it.
#[rupa::repository]
pub trait Notes: Send + Sync {
    #[query(insert)]
    async fn add(&self, note: &NewNote) -> Result<u64, DynError>;

    #[query(filter = title == $title)]
    async fn by_title(&self, title: &str) -> Result<Option<Note>, DynError>;

    #[query(order_by = id)]
    async fn all(&self) -> Result<Vec<Note>, DynError>;
}

/// Runs the whole suite through `db`; see the [module docs](self). `ddl`
/// resets the schema before each case.
pub async fn run_database(db: &Database, ddl: &'static str) {
    {
        let mut h = DatabaseHarness::new(db, ddl).await;
        crate::run(&mut h).await;
        crate::run_transactions(&mut h).await;
    }
    run_pool_cases(async || {
        run_script(&mut db.get().await.unwrap(), ddl).await;
        db.clone()
    })
    .await;
}

/// The cases that use a [`Database`]'s pool. `fresh` returns a database with
/// empty conformance tables, each time it is called.
pub async fn run_pool_cases(mut fresh: impl AsyncFnMut() -> Database) {
    macro_rules! cases {
        ($($case:ident),* $(,)?) => {$(
            let db = fresh().await;
            eprintln!("database {}", stringify!($case));
            $case(&db).await;
        )*};
    }
    cases!(
        db_owned_tx_commits_and_rolls_back,
        db_dropped_owned_tx_rolls_back_before_reuse,
        db_repository_over_the_pool,
        db_secure_tx_is_checked_at_run_time,
        db_connections_return_to_the_pool,
    );
}

async fn add<E: AsyncExecutor>(e: &mut E, note: &NewNote) {
    let n = e.run(insert::<Note>().value(note).affected()).await;
    assert_eq!(n.unwrap(), 1);
}

fn note(title: &str) -> NewNote {
    NewNote {
        title: title.into(),
        pinned: false,
    }
}

async fn titles(db: &Database) -> Vec<String> {
    let notes = Arc::new(Repo::new(db.clone())) as Arc<dyn Notes>;
    notes
        .all()
        .await
        .unwrap()
        .into_iter()
        .map(|n| n.title)
        .collect()
}

/// Nested savepoints work on owned transactions too.
async fn db_owned_tx_commits_and_rolls_back(db: &Database) {
    let mut tx = db.begin().await.unwrap();
    add(&mut tx, &note("kept")).await;
    tx.commit().await.unwrap();

    let mut tx = db.begin().await.unwrap();
    add(&mut tx, &note("gone")).await;
    let mut inner = tx.begin().await.unwrap();
    add(&mut inner, &note("inner")).await;
    inner.commit().await.unwrap();
    tx.rollback().await.unwrap();

    assert_eq!(titles(db).await, ["kept"]);
}

/// Dropped while open: the connection goes back to the pool owing a
/// rollback, which must happen before anyone else uses it.
async fn db_dropped_owned_tx_rolls_back_before_reuse(db: &Database) {
    {
        let mut tx = db.begin().await.unwrap();
        add(&mut tx, &note("gone")).await;
    }
    let mut tx = db.begin().await.unwrap();
    add(&mut tx, &note("next")).await;
    tx.commit().await.unwrap();
    assert_eq!(titles(db).await, ["next"]);
}

async fn db_repository_over_the_pool(db: &Database) {
    let notes: Arc<dyn Notes> = Arc::new(Repo::new(db.clone()));
    assert_eq!(notes.add(&note("a")).await.unwrap(), 1);
    assert_eq!(notes.add(&note("b")).await.unwrap(), 1);
    assert_eq!(notes.by_title("b").await.unwrap().unwrap().title, "b");
    assert!(notes.by_title("zzz").await.unwrap().is_none());
}

async fn db_secure_tx_is_checked_at_run_time(db: &Database) {
    let result = db
        .try_begin_with_security(
            TxOptions::default(),
            SecurityContext::new().set("tenant_id", "1"),
        )
        .await;
    if db.dialect().supports(Capability::NativeRls) {
        let tx = result.expect("native RLS is supported");
        assert_eq!(tx.context().get("tenant_id"), Some("1"));
        tx.rollback().await.unwrap();
    } else {
        let err = result.expect_err("no native RLS: refused");
        assert_eq!(
            err.downcast_ref::<TxError>(),
            Some(&TxError::Unsupported {
                dialect: db.dialect().id(),
                capability: Capability::NativeRls,
            })
        );
    }
}

/// One at a time: a single-connection database has only one.
async fn db_connections_return_to_the_pool(db: &Database) {
    let before = db.status();
    {
        let _conn = db.get().await.unwrap();
        assert_eq!(db.status().in_use(), before.in_use() + 1);
    }
    {
        let _tx = db.begin().await.unwrap();
        assert_eq!(db.status().in_use(), before.in_use() + 1);
    }
    assert_eq!(db.status().in_use(), before.in_use(), "{:?}", db.status());
}
