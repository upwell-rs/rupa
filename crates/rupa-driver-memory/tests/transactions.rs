//! Sync transaction API specifics: the closure form and panics.

use rupa::core::exec::Executor;
use rupa::core::tx::{Transaction, Transactional};
use rupa::prelude::*;
use rupa_conformance::{NewNote, Note};
use rupa_driver_memory::{MemoryDb, MemoryError};

fn db() -> MemoryDb {
    let mut db = MemoryDb::new();
    db.register::<Note>();
    db
}

fn add<E: Executor>(e: &mut E, title: &str) -> Result<u64, E::Error> {
    e.run(
        insert::<Note>()
            .value(&NewNote {
                title: title.into(),
                pinned: false,
            })
            .affected(),
    )
}

fn count(db: &mut MemoryDb) -> usize {
    db.run(select::<Note>().all()).unwrap().len()
}

#[test]
fn closure_commits_on_ok_and_rolls_back_on_err() {
    let mut db = db();
    let n = db.transaction(|tx| add(tx, "kept")).unwrap();
    assert_eq!(n, 1);

    let r: Result<(), MemoryError> = db.transaction(|tx| {
        add(tx, "dropped")?;
        Err(MemoryError::Unsupported("abort".into()))
    });
    assert!(r.is_err());
    assert_eq!(count(&mut db), 1);
}

#[test]
fn nested_closures_use_savepoints() {
    let mut db = db();
    db.transaction(|outer| {
        add(outer, "outer")?;
        let inner: Result<(), MemoryError> = outer.transaction(|inner| {
            add(inner, "inner")?;
            Err(MemoryError::Unsupported("inner abort".into()))
        });
        assert!(inner.is_err());
        Ok::<_, MemoryError>(())
    })
    .unwrap();
    let titles: Vec<_> = db
        .run(select::<Note>().all())
        .unwrap()
        .into_iter()
        .map(|n| n.title)
        .collect();
    assert_eq!(titles, ["outer"]);
}

#[test]
fn panic_inside_a_transaction_rolls_back() {
    let mut db = db();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _ = db.transaction(|tx| -> Result<(), MemoryError> {
            add(tx, "doomed")?;
            panic!("boom");
        });
    }));
    assert!(result.is_err());
    assert_eq!(
        count(&mut db),
        0,
        "unwinding drops the tx, which rolls back"
    );
}

#[test]
fn guard_api_commit() {
    let mut db = db();
    let mut tx = db.begin().unwrap();
    add(&mut tx, "a").unwrap();
    tx.commit().unwrap();
    assert_eq!(count(&mut db), 1);
}
