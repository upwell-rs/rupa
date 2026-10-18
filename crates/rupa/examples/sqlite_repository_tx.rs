//! Two repositories sharing one application-owned SQLite transaction.
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

    let session = rupa::TxSession::new(NoteWritesView::new(&pool).begin()?);
    let writes = NoteWritesView::new(&session);
    let reads = NoteReadsView::new(&session);
    assert_eq!(writes.add(&note)?, 1);
    assert_eq!(reads.all()?[0].title, "shared");
    session.rollback()?;
    assert!(writes.add(&note).is_err());
    assert!(NoteReadsView::new(&pool).all()?.is_empty());

    let session = rupa::TxSession::new(NoteWritesView::new(&pool).begin()?);
    NoteWritesView::new(&session).add(&note)?;
    session.commit()?;
    assert_eq!(NoteReadsView::new(&pool).all()?.len(), 1);

    println!("two repository views shared one transaction; rollback and commit passed");
    Ok(())
}
