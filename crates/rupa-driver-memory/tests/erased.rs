//! Executors chosen at run time: boxed, with `Dialect = DynDialect`.

use rupa::prelude::*;
use rupa_conformance::{Item, items};
use rupa_core::dialect::{DialectId, Supports, caps};
use rupa_core::exec::{AsyncExecutor, BoxAsyncExecutor, BoxExecutor, ExecError, Executor};
use rupa_core::{Dialect, ResultError};
use rupa_driver_memory::MemoryDb;

fn seeded() -> MemoryDb {
    let mut db = MemoryDb::new();
    db.register::<Item>();
    Executor::run(&mut db, insert::<Item>().values(&items()).affected()).unwrap();
    db
}

#[test]
fn boxed_sync_executor_runs_queries_and_keeps_result_errors() {
    let mut exec = BoxExecutor::new(seeded());
    assert_eq!(exec.dialect().id(), DialectId::Memory);
    let all = exec
        .run(select::<Item>().order_by(col!(Item::id).asc()).all())
        .unwrap();
    assert_eq!(all, items());

    let err = exec
        .run(select::<Item>().filter(col!(Item::id).eq(99)).one())
        .unwrap_err();
    assert_eq!(err.result_error(), Some(&ResultError::NotFound));
}

#[test]
fn boxed_async_executor_runs_queries() {
    let mut exec = BoxAsyncExecutor::new(seeded().into_async());
    let n = pollster::block_on(
        exec.run(
            delete::<Item>()
                .filter(col!(Item::active).eq(true))
                .affected(),
        ),
    )
    .unwrap();
    assert_eq!(n, 3);
    assert!(
        pollster::block_on(exec.run(select::<Item>().filter(col!(Item::id).eq(2)).exists()))
            .unwrap()
    );
}

/// Statically gated code accepts the concrete memory executor.
fn needs_json_paths<E: Executor>(e: &mut E) -> usize
where
    E::Dialect: Supports<caps::JsonPath>,
{
    e.run(
        select::<Item>()
            .filter(col!(Item::meta).path("flag").cast::<bool>().eq(true))
            .all(),
    )
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
