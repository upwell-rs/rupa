//! `Query::execute`: the engine decides whether the result comes back
//! directly or as a future to `.await`.

use rupa::prelude::*;
use rupa_conformance::{Item, items};
use rupa_core::erased::{BoxAsyncExecutor, BoxExecutor};
use rupa_core::exec::{Blocking, ExecError};
use rupa_core::{AsyncPool, ResultError, SingleConnection};
use rupa_driver_memory::MemoryDb;

fn seeded() -> MemoryDb {
    let mut db = MemoryDb::new();

    db.register::<Item>();
    insert::<Item>()
        .values(&items())
        .affected()
        .execute(&mut db)
        .expect("items are seeded");

    db
}

#[test]
fn a_sync_engine_returns_the_result() {
    let mut db = seeded();

    let all = select::<Item>()
        .order_by(Item::ID.asc())
        .all()
        .execute(&mut db)
        .expect("select runs");
    let missing = select::<Item>()
        .filter(Item::ID.eq(99))
        .one()
        .execute(&mut db)
        .expect_err("no item 99");

    assert_eq!(all, items());
    assert_eq!(missing.result_error(), Some(&ResultError::NotFound));
}

#[test]
fn an_async_engine_returns_a_future() {
    let mut db = seeded().into_async();

    let removed = pollster::block_on(
        delete::<Item>()
            .filter(Item::ACTIVE.eq(true))
            .affected()
            .execute(&mut db),
    )
    .expect("delete runs");
    let left = pollster::block_on(select::<Item>().all().execute(&mut db)).expect("select runs");

    assert_eq!(removed, 3);
    assert_eq!(left.len(), items().len() - 3);
}

#[test]
fn a_blocking_sync_engine_is_awaited() {
    let mut db = Blocking(seeded());

    let exists = pollster::block_on(
        select::<Item>()
            .filter(Item::ID.eq(2))
            .exists()
            .execute(&mut db),
    )
    .expect("exists runs");

    assert!(exists);
}

#[test]
fn erased_and_pooled_engines_keep_their_mode() {
    let mut boxed = BoxExecutor::new(seeded());
    let mut boxed_async = BoxAsyncExecutor::new(seeded().into_async());
    let pool = SingleConnection::new(seeded().into_async());
    let mut pooled = pollster::block_on(pool.get()).expect("the connection is free");

    let sync = select::<Item>().all().execute(&mut boxed).expect("runs");
    let awaited =
        pollster::block_on(select::<Item>().all().execute(&mut boxed_async)).expect("runs");
    let through_pool =
        pollster::block_on(select::<Item>().all().execute(&mut pooled)).expect("runs");

    assert_eq!(sync.len(), items().len());
    assert_eq!(awaited.len(), items().len());
    assert_eq!(through_pool.len(), items().len());
}
