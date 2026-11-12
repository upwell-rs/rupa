//! Declared relations, left joins, merged rows and SQL parameters.
//!
//! Run: `cargo run -p rupa --example sqlite_joins --features sqlite`

#[path = "support/sqlite.rs"]
mod sqlite;

use rupa::prelude::*;

#[derive(Debug, Clone, PartialEq, Entity, Insertable)]
#[entity(table = "authors")]
struct Author {
    #[id]
    id: i64,
    name: String,
}

#[derive(Debug, Clone, PartialEq, Entity, Insertable)]
#[entity(table = "books")]
struct Book {
    #[id]
    id: i64,
    #[column(references = Author)]
    author_id: i64,
    title: String,
}

#[derive(Debug, PartialEq, Joined)]
struct BookWithAuthor {
    #[from(Book::title)]
    title: String,
    #[from(Author::name)]
    author: String,
}

#[rupa::query(entity = Book, inner_join = Author, filter = Author::name == $name)]
fn books_by(name: &str) -> Vec<BookWithAuthor>;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pool = sqlite::open("CREATE TABLE authors (id INTEGER PRIMARY KEY, name TEXT NOT NULL)")?;
    raw().sql("CREATE TABLE books (id INTEGER PRIMARY KEY, author_id INTEGER NOT NULL, title TEXT NOT NULL)").execute::<u64>().execute(&pool)?;
    insert::<Author>()
        .values(&[
            Author {
                id: 1,
                name: "Ada".into(),
            },
            Author {
                id: 2,
                name: "Grace".into(),
            },
        ])
        .affected()
        .execute(&pool)?;
    insert::<Book>()
        .value(&Book {
            id: 10,
            author_id: 1,
            title: "Notes".into(),
        })
        .affected()
        .execute(&pool)?;

    let joined = select::<Author>()
        .join::<Book>()
        .left()
        .all()
        .execute(&pool)?;
    assert_eq!(joined.len(), 2);
    assert_eq!(
        joined[0].1.as_ref().map(|book| book.title.as_str()),
        Some("Notes")
    );
    assert!(joined[1].1.is_none());

    let reverse = select::<Book>()
        .join::<Author>()
        .inner_via(Book::author())
        .all()
        .execute(&pool)?;
    assert_eq!(reverse[0].1.name, "Ada");

    let by_ada = books_by("Ada").execute(&pool)?;
    assert_eq!(
        by_ada,
        [BookWithAuthor {
            title: "Notes".into(),
            author: "Ada".into()
        }]
    );

    let sql = rupa::sql::render_query(&books_by("Ada"), &rupa::Sqlite)?;
    assert!(sql.sql.contains("JOIN"));
    assert!(!sql.sql.contains("'Ada'"));
    assert_eq!(sql.params.len(), 1);

    println!("left join, declared query, merged row and bound SQL passed");
    Ok(())
}
