//! Typed CRUD and transactions against SQLite in memory.
//!
//! Run: `cargo run -p rupa --example sqlite_crud --features sqlite`

#[path = "support/sqlite.rs"]
mod sqlite;
#[path = "support/task.rs"]
mod task;

use rupa::Pool;
use rupa::prelude::*;
use task::{NewTask, Task, TaskPatch};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pool = sqlite::open(task::SCHEMA)?;
    let mut conn = pool.get()?;
    let task = insert::<Task>()
        .value(&NewTask {
            title: "write examples".into(),
            done: false,
        })
        .returning_one()
        .execute(&mut conn)?;
    assert_eq!(
        get::<Task>(&task.id).execute(&mut conn)?,
        Some(task.clone())
    );

    update::<Task>()
        .one(&TaskPatch {
            id: task.id,
            done: Some(true),
        })
        .affected()
        .execute(&mut conn)?;
    assert_eq!(
        select::<Task>()
            .filter(Task::DONE.eq(true))
            .all()
            .execute(&mut conn)?
            .len(),
        1
    );

    {
        let mut tx = conn.begin()?;
        delete::<Task>()
            .by_id(&task.id)
            .affected()
            .execute(&mut tx)?;
    }
    assert!(get::<Task>(&task.id).execute(&mut conn)?.is_some());

    let removed = conn.transaction(|tx| delete::<Task>().by_id(&task.id).affected().execute(tx))?;
    assert_eq!(removed, 1);
    assert!(get::<Task>(&task.id).execute(&mut conn)?.is_none());

    println!("insert, get, filter, patch, rollback, commit and delete passed");
    Ok(())
}
