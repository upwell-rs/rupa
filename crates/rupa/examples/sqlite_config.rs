//! Configuration-driven SQLite pool, raw schema setup and typed repository.
//!
//! Run: `cargo run -p rupa --example sqlite_config --features sqlite,dynamic`

use rupa::prelude::*;
use rupa::{DbConfig, DynError};

#[derive(Debug, Clone, Entity, Insertable)]
#[entity(table = "notes")]
struct Note {
    #[id]
    id: i64,
    title: String,
}

#[repository]
trait Notes: Send + Sync {
    #[query(filter = id == $id)]
    async fn by_id(&self, id: i64) -> Result<Option<Note>, DynError>;
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    pollster::block_on(async {
        let config = DbConfig::new("sqlite::memory:");
        let db = rupa::connect(&config).await?;
        let mut conn = db.get().await?;
        raw()
            .sql("CREATE TABLE notes (id INTEGER PRIMARY KEY, title TEXT NOT NULL)")
            .execute::<u64>()
            .execute(&mut conn)
            .await?;
        insert::<Note>()
            .value(&Note {
                id: 1,
                title: "pooled".into(),
            })
            .affected()
            .execute(&mut conn)
            .await?;
        drop(conn);

        let notes = NotesView::new(&db);
        let found = notes.by_id(1).await?.expect("note 1 exists");
        assert_eq!(found.title, "pooled");
        assert_eq!(db.status().max_size, 1);

        println!("configured SQLite, raw DDL, pool and repository passed");
        Ok::<(), Box<dyn std::error::Error>>(())
    })
}
