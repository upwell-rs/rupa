//! One repository that writes as well as reads: inserts (with and without
//! `returning`, one and many), get, a patch update, delete and a filtered
//! select, writing through subset shapes (`NewBook`, `BookPatch`), plus two
//! the repository branching into a transaction that a second one joins.
//!
//! Run: `cargo run -p rupa --example sqlite_repository_crud --features sqlite`

#[path = "support/sqlite.rs"]
mod sqlite;

use rupa::prelude::*;

#[derive(Debug, Clone, PartialEq, Entity, Gettable, Insertable, Updatable, Deletable)]
#[entity(table = "books")]
struct Book {
    #[id(generated)]
    id: i64,
    title: String,
    author: String,
    year: i32,
    read: bool,
}

/// What a new book needs: everything but the generated id.
#[derive(Insertable)]
#[insertable(entity = Book)]
struct NewBook {
    title: String,
    author: String,
    year: i32,
    read: bool,
}

/// A partial update: `None` leaves a column alone.
#[derive(Updatable)]
#[updatable(entity = Book)]
struct BookPatch {
    #[id]
    id: i64,
    read: Option<bool>,
}

#[repository]
trait Books {
    #[query(insert)]
    fn add(&self, book: &NewBook) -> Result<u64, DynError>;

    #[query(insert, returning)]
    fn create(&self, book: &NewBook) -> Result<Book, DynError>;

    #[query(insert, returning)]
    fn create_many(&self, books: &[NewBook]) -> Result<Vec<Book>, DynError>;

    #[query(get)]
    fn get(&self, id: i64) -> Result<Option<Book>, DynError>;

    #[query(update)]
    fn patch(&self, patch: &BookPatch) -> Result<bool, DynError>;

    #[query(delete, entity = Book)]
    fn remove(&self, id: i64) -> Result<u64, DynError>;

    #[query(filter = author == $author && read == false, order_by = year)]
    fn unread_by(&self, author: &str) -> Result<Vec<Book>, DynError>;
}

#[repository]
trait Shelf {
    #[query(filter = read == true, order_by = id)]
    fn finished(&self) -> Result<Vec<Book>, DynError>;
}

fn new_book(title: &str, author: &str, year: i32) -> NewBook {
    NewBook {
        title: title.into(),
        author: author.into(),
        year,
        read: false,
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pool = sqlite::open(
        "CREATE TABLE books (id INTEGER PRIMARY KEY, title TEXT NOT NULL, author TEXT NOT NULL, year INTEGER NOT NULL, read BOOLEAN NOT NULL)",
    )?;
    let books = BooksRepo::new(&pool);

    let dune = books.create(&new_book("Dune", "Herbert", 1965))?;
    let more = books.create_many(&[
        new_book("Children of Dune", "Herbert", 1976),
        new_book("Dune Messiah", "Herbert", 1969),
    ])?;

    assert_eq!(dune.id, 1, "the database generated the id");
    assert_eq!(more.iter().map(|b| b.id).collect::<Vec<_>>(), [2, 3]);
    assert_eq!(books.add(&new_book("Hyperion", "Simmons", 1989))?, 1);
    assert_eq!(books.get(dune.id)?.map(|b| b.title), Some("Dune".into()));

    assert!(books.patch(&BookPatch {
        id: dune.id,
        read: Some(true),
    })?);
    assert_eq!(
        books
            .unread_by("Herbert")?
            .iter()
            .map(|b| b.title.as_str())
            .collect::<Vec<_>>(),
        ["Dune Messiah", "Children of Dune"]
    );

    let shelf = ShelfRepo::new(&pool);
    let tx = books.open_tx(None)?;

    tx.patch(&BookPatch {
        id: more[0].id,
        read: Some(true),
    })?;
    assert_eq!(
        shelf.tx(tx.connection()).finished()?.len(),
        2,
        "sees the transaction's write"
    );
    tx.rollback()?;
    assert_eq!(shelf.finished()?.len(), 1, "rolled back");

    assert_eq!(books.remove(dune.id)?, 1);
    assert!(books.get(dune.id)?.is_none());

    println!(
        "repository inserts, get, patch, filtered select, shared transaction and delete passed"
    );
    Ok(())
}
