//! Typed CRUD and transactions against SQLite in memory.
//!
//! Run: `cargo run -p rupa --example sqlite_crud --features sqlite`

#[path = "support/sqlite.rs"]
mod sqlite;
#[path = "support/task.rs"]
mod task;

use rupa::prelude::*;
use rupa::{Executor, Pool};
use task::{NewTask, Task, TaskPatch};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pool = sqlite::open(task::SCHEMA)?;
    let mut conn = pool.get()?;
    let task = conn.run(
        insert::<Task>()
            .value(&NewTask {
                title: "write examples".into(),
                done: false,
            })
            .returning_one(),
    )?;
    assert_eq!(conn.run(get::<Task>(&task.id))?, Some(task.clone()));

    conn.run(
        update::<Task>()
            .one(&TaskPatch {
                id: task.id,
                done: Some(true),
            })
            .affected(),
    )?;
    assert_eq!(
        conn.run(select::<Task>().filter(col!(Task::done).eq(true)).all())?
            .len(),
        1
    );

    {
        let mut tx = conn.begin()?;
        tx.run(delete::<Task>().by_id(&task.id).affected())?;
    }
    assert!(conn.run(get::<Task>(&task.id))?.is_some());

    let removed = conn.transaction(|tx| tx.run(delete::<Task>().by_id(&task.id).affected()))?;
    assert_eq!(removed, 1);
    assert!(conn.run(get::<Task>(&task.id))?.is_none());

    println!("insert, get, filter, patch, rollback, commit and delete passed");
    Ok(())
}
