//! The same Task model and queries as `sqlite_crud`, on an async executor.
//!
//! Run: `cargo run -p rupa --example sqlite_async --features sqlx-sqlite`

#[path = "support/task.rs"]
mod task;

use rupa::drivers::sqlx::sqlite::SqlitePool;
use rupa::prelude::*;
use rupa::{AsyncExecutor, AsyncPool, DbConfig};
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
    let mut conn = pool.get().await?;
    conn.run(raw().sql(task::SCHEMA).execute::<u64>()).await?;
    drop(conn);

    let session = TasksView::new(&pool).begin_async().await?;
    let tasks = TasksView::new(&session);
    tasks
        .add(&NewTask {
            title: "write examples".into(),
            done: false,
        })
        .await?;
    assert_eq!(
        tasks.get(1).await?.expect("task in transaction").title,
        "write examples"
    );
    session.commit().await?;
    assert!(
        tasks.get(1).await.is_err(),
        "the finished session stays closed"
    );

    let mut conn = pool.get().await?;
    let task = conn.run(get::<Task>(&1)).await?.expect("task 1 exists");
    assert_eq!(task.title, "write examples");
    conn.run(
        update::<Task>()
            .one(&TaskPatch {
                id: task.id,
                done: Some(true),
            })
            .affected(),
    )
    .await?;
    drop(conn);
    assert!(
        TasksView::new(&pool)
            .get(task.id)
            .await?
            .expect("task still exists")
            .done
    );

    println!("same Task queries on an async executor passed");
    Ok(())
}
