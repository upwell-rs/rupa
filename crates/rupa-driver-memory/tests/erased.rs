//! Executors chosen at run time: boxed, with `Dialect = DynDialect`.

use rupa::prelude::*;
use rupa_conformance::{Item, items};
use rupa_core::dialect::{DialectId, Supports, caps};
use rupa_core::erased::{BoxAsyncExecutor, BoxExecutor};
use rupa_core::exec::{ExecError, Executor};
use rupa_core::{Dialect, ResultError};
use rupa_driver_memory::MemoryDb;

fn seeded() -> MemoryDb {
    let mut db = MemoryDb::new();
    db.register::<Item>();
    insert::<Item>()
        .values(&items())
        .affected()
        .execute(&mut db)
        .unwrap();
    db
}

#[test]
fn boxed_sync_executor_runs_queries_and_keeps_result_errors() {
    let mut exec = BoxExecutor::new(seeded());
    assert_eq!(exec.dialect().id(), DialectId::Memory);
    let all = select::<Item>()
        .order_by(Item::ID.asc())
        .all()
        .execute(&mut exec)
        .unwrap();
    assert_eq!(all, items());

    let err = select::<Item>()
        .filter(Item::ID.eq(99))
        .one()
        .execute(&mut exec)
        .unwrap_err();
    assert_eq!(err.result_error(), Some(&ResultError::NotFound));
}

#[test]
fn boxed_async_executor_runs_queries() {
    let mut exec = BoxAsyncExecutor::new(seeded().into_async());
    let n = pollster::block_on(
        delete::<Item>()
            .filter(Item::ACTIVE.eq(true))
            .affected()
            .execute(&mut exec),
    )
    .unwrap();
    assert_eq!(n, 3);
    assert!(
        pollster::block_on(
            select::<Item>()
                .filter(Item::ID.eq(2))
                .exists()
                .execute(&mut exec)
        )
        .unwrap()
    );
}

/// Statically gated code accepts the concrete memory executor.
fn needs_json_paths<E: Executor>(e: &mut E) -> usize
where
    E::Dialect: Supports<caps::JsonPath>,
{
    select::<Item>()
        .filter(Item::META.path("flag").cast::<bool>().eq(true))
        .all()
        .execute(e)
        .unwrap()
        .len()
}

#[test]
fn static_capability_bound_on_executor_dialect() {
    assert_eq!(needs_json_paths(&mut seeded()), 2);
    // The boxed executor is runtime-checked instead.
    assert!(
        BoxExecutor::new(seeded())
            .dialect()
            .supports(rupa_core::Capability::JsonPath)
    );
}
