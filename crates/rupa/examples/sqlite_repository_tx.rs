//! Two repositories sharing one application-owned SQLite transaction: each
//! holds `&tx`, so neither needs a session type, and `commit` takes the
//! transaction only once both are done with it.
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

#[repository]
trait NoteWrites {
    #[query(insert)]
    fn add(&self, note: &Note) -> Result<u64, rupa::DynError>;
}

#[repository]
trait NoteReads {
    #[query(order_by = id)]
    fn all(&self) -> Result<Vec<Note>, rupa::DynError>;
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pool = sqlite::open("CREATE TABLE notes (id INTEGER PRIMARY KEY, title TEXT NOT NULL)")?;
    let note = Note {
        id: 1,
        title: "shared".into(),
    };

    let tx = pool.begin()?;
    assert_eq!(NoteWritesRepo::new(&tx).add(&note)?, 1);
    assert_eq!(NoteReadsRepo::new(&tx).all()?[0].title, "shared");
    tx.rollback()?;
    assert!(NoteReadsRepo::new(&pool).all()?.is_empty());

    let tx = pool.begin()?;
    NoteWritesRepo::new(&tx).add(&note)?;
    tx.commit()?;
    assert_eq!(NoteReadsRepo::new(&pool).all()?.len(), 1);

    println!("two repositories shared one transaction; rollback and commit passed");
    Ok(())
}
