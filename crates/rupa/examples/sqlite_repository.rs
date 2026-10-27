//! A repository trait using a SQLite pool and static dispatch.
//!
//! Run: `cargo run -p rupa --example sqlite_repository --features sqlite`

#[path = "support/sqlite.rs"]
mod sqlite;

use rupa::Pool;
use rupa::prelude::*;

#[derive(Debug, Clone, Entity, Insertable)]
#[entity(table = "messages")]
struct Message {
    #[id]
    id: i64,
    body: String,
    read: bool,
}

#[repository]
trait Messages {
    #[query(filter = id == $id)]
    fn by_id(&self, id: i64) -> Result<Option<Message>, rupa::DynError>;

    #[query(filter = read == $read, order_by = id)]
    fn with_read_state(&self, read: bool) -> Result<Vec<Message>, rupa::DynError>;
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pool = sqlite::open(
        "CREATE TABLE messages (id INTEGER PRIMARY KEY, body TEXT NOT NULL, read BOOLEAN NOT NULL)",
    )?;
    insert::<Message>()
        .values(&[
            Message {
                id: 1,
                body: "hello".into(),
                read: false,
            },
            Message {
                id: 2,
                body: "bye".into(),
                read: true,
            },
        ])
        .affected()
        .execute(&mut pool.get()?)?;

    let messages = MessagesView::new(&pool);
    let first = messages.by_id(1)?.expect("message 1 exists");
    assert_eq!(first.body, "hello");
    assert_eq!(messages.with_read_state(true)?[0].id, 2);

    println!("static repository with pooled SQLite connections passed");
    Ok(())
}
