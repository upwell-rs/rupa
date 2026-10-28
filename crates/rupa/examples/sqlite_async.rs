//! The same Task model and queries as `sqlite_crud`, on an async pool: a
//! repository over the pool, branching into a transaction with `open_tx`.
//!
//! Run: `cargo run -p rupa --example sqlite_async --features sqlx-sqlite`

#[path = "support/task.rs"]
mod task;

use rupa::drivers::sqlx::sqlite::SqlitePool;
use rupa::prelude::*;
use rupa::{AsyncPool, DbConfig};
use task::{NewTask, Task, TaskPatch};

#[repository]
trait Tasks {
    #[query(insert)]
    async fn add(&self, task: &NewTask) -> Result<u64, rupa::DynError>;

    #[query(get)]
    async fn get(&self, id: i64) -> Result<Option<Task>, rupa::DynError>;
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pool = SqlitePool::connect(&DbConfig::new("sqlite::memory:")).await?;
    raw()
        .sql(task::SCHEMA)
        .execute::<u64>()
        .execute(&pool)
        .await?;
    let tasks = TasksRepo::new(&pool);
    let tx = tasks.open_tx(None).await?;

    tx.add(&NewTask {
        title: "write examples".into(),
        done: false,
    })
    .await?;
    assert_eq!(
        tx.get(1).await?.expect("task in transaction").title,
        "write examples"
    );
    tx.commit().await?;

    let task = tasks.get(1).await?.expect("committed task");
    assert_eq!(task.title, "write examples");
    update::<Task>()
        .one(&TaskPatch {
            id: task.id,
            done: Some(true),
        })
        .affected()
        .execute(&pool)
        .await?;
    assert!(tasks.get(task.id).await?.expect("task still exists").done);
    assert_eq!(pool.status().max_size, 1);

    println!("same Task queries on an async pool passed");
    Ok(())
}
