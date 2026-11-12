//! Typed CRUD and transactions against SQLite in memory.
//!
//! Run: `cargo run -p rupa --example sqlite_crud --features sqlite`

#[path = "support/sqlite.rs"]
mod sqlite;
#[path = "support/task.rs"]
mod task;

use rupa::prelude::*;
use task::{NewTask, Task, TaskPatch};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pool = sqlite::open(task::SCHEMA)?;
    let task = insert::<Task>()
        .value(&NewTask {
            title: "write examples".into(),
            done: false,
        })
        .returning_one()
        .execute(&pool)?;
    assert_eq!(get::<Task>(&task.id).execute(&pool)?, Some(task.clone()));

    update::<Task>()
        .one(&TaskPatch {
            id: task.id,
            done: Some(true),
        })
        .affected()
        .execute(&pool)?;
    assert_eq!(
        select::<Task>()
            .filter(Task::DONE.eq(true))
            .all()
            .execute(&pool)?
            .len(),
        1
    );

    {
        let tx = pool.begin()?;
        delete::<Task>().by_id(&task.id).affected().execute(&tx)?;
    }
    assert!(get::<Task>(&task.id).execute(&pool)?.is_some());

    let removed = pool.transaction(|tx| delete::<Task>().by_id(&task.id).affected().execute(tx))?;
    assert_eq!(removed, 1);
    assert!(get::<Task>(&task.id).execute(&pool)?.is_none());

    println!("insert, get, filter, patch, rollback, commit and delete passed");
    Ok(())
}
