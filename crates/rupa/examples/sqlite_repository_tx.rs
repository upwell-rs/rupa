//! One repository branching into a transaction and back: `open_tx` is the
//! same repository on a new transaction, `tx` puts another repository on it,
//! and `commit` or `rollback` ends it.
//!
//! Run: `cargo run -p rupa --example sqlite_repository_tx --features sqlite`

#[path = "support/sqlite.rs"]
mod sqlite;

use rupa::prelude::*;

#[derive(Debug, Clone, Entity, Insertable)]
#[entity(table = "notes")]
struct Note {
    #[id]
    id: i64,
    title: String,
}

#[derive(Debug, Clone, Entity, Insertable)]
#[entity(table = "tags")]
struct Tag {
    #[id]
    id: i64,
    note_id: i64,
    name: String,
}

#[repository]
trait Notes {
    #[query(insert)]
    fn add(&self, note: &Note) -> Result<u64, DynError>;

    #[query(order_by = id)]
    fn all(&self) -> Result<Vec<Note>, DynError>;
}

#[repository]
trait Tags {
    #[query(insert)]
    fn add(&self, tag: &Tag) -> Result<u64, DynError>;

    #[query(filter = note_id == $note_id)]
    fn of(&self, note_id: i64) -> Result<Vec<Tag>, DynError>;
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pool = sqlite::open(
        "CREATE TABLE notes (id INTEGER PRIMARY KEY, title TEXT NOT NULL);
         CREATE TABLE tags (id INTEGER PRIMARY KEY, note_id INTEGER NOT NULL, name TEXT NOT NULL)",
    )?;
    let notes = NotesRepo::new(&pool);
    let tags = TagsRepo::new(&pool);
    let note = Note {
        id: 1,
        title: "shared".into(),
    };
    let tag = Tag {
        id: 1,
        note_id: 1,
        name: "draft".into(),
    };

    let tx = notes.open_tx(None)?;

    tx.add(&note)?;
    assert_eq!(tx.all()?[0].title, "shared", "sees its own write");
    tx.rollback()?;
    assert!(notes.all()?.is_empty(), "rolled back");

    let tx = notes.open_tx(None)?;
    let tagged = tags.tx(tx.connection());

    tx.add(&note)?;
    tagged.add(&tag)?;
    assert_eq!(
        tagged.of(1)?.len(),
        1,
        "the tags share the note's transaction"
    );
    tx.commit()?;
    assert_eq!(notes.all()?.len(), 1);
    assert_eq!(tags.of(1)?[0].name, "draft");

    println!("one repository opened, shared, rolled back and committed a transaction");
    Ok(())
}
