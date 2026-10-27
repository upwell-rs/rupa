//! The same Task model and queries as `sqlite_crud`, on an async executor.
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
    async fn add(&mut self, task: &NewTask) -> Result<u64, rupa::DynError>;

    #[query(get)]
    async fn get(&mut self, id: i64) -> Result<Option<Task>, rupa::DynError>;
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pool = SqlitePool::connect(&DbConfig::new("sqlite::memory:")).await?;
    let mut conn = pool.get().await?;
    raw()
        .sql(task::SCHEMA)
        .execute::<u64>()
        .execute(&mut conn)
        .await?;
    let mut repository = TasksRepo::new(conn);
    let mut tx = repository.begin().await?;
    let mut tasks = TasksView::new(&mut tx);
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
    tx.commit().await?;

    let task = repository.get(1).await?.expect("committed task");
    let mut conn = repository.into_inner();
    assert_eq!(task.title, "write examples");
    update::<Task>()
        .one(&TaskPatch {
            id: task.id,
            done: Some(true),
        })
        .affected()
        .execute(&mut conn)
        .await?;
    assert!(
        TasksView::new(&mut conn)
            .get(task.id)
            .await?
            .expect("task still exists")
            .done
    );

    println!("same Task queries on an async executor passed");
    Ok(())
}
