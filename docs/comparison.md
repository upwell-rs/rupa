# rupa, Diesel and SeaORM

The same small tasks in each: a repository with CRUD, two repositories
sharing a transaction, a join, a JSON filter, and a unit test without a
database. The rupa snippets are taken from the runnable examples. The Diesel
(2.3) and SeaORM (2.0) snippets are **illustrative**: written against their
documented APIs at those releases, not compiled in this repository, and
trimmed of imports and schema declarations.

## A repository with CRUD

**rupa**: declare the trait; the macro writes the queries and one repository
type over any connection ([full example](../crates/rupa/examples/sqlite_repository_crud.rs)).

```rust
#[repository]
trait Books {
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

let books = BooksRepo::new(&pool);
let dune = books.create(&new_book)?;
```

**Diesel**: queries are written where they are used, or wrapped by hand.

```rust
let dune: Book = diesel::insert_into(books::table)
    .values(&new_book)
    .get_result(&mut conn)?;
let unread: Vec<Book> = books::table
    .filter(books::author.eq("Herbert").and(books::read.eq(false)))
    .order(books::year)
    .load(&mut conn)?;
```

**SeaORM**: entities and active models carry the operations.

```rust
let dune = book::ActiveModel {
    title: Set("Dune".to_owned()),
    author: Set("Herbert".to_owned()),
    ..Default::default()
}
.insert(&db)
.await?;
let unread = Book::find()
    .filter(book::Column::Author.eq("Herbert"))
    .filter(book::Column::Read.eq(false))
    .order_by_asc(book::Column::Year)
    .all(&db)
    .await?;
```

## Two repositories, one transaction

**rupa**: a transaction is a connection; repositories borrow it, and
`commit` cannot run while they still do
([example](../crates/rupa/examples/sqlite_repository_tx.rs)).

```rust
let tx = pool.begin()?;
NoteWritesRepo::new(&tx).add(&note)?;
NoteReadsRepo::new(&tx).all()?;
tx.commit()?;
```

**Diesel**: a closure receives the connection; everything inside runs in the
transaction.

```rust
conn.transaction(|conn| {
    diesel::insert_into(notes::table).values(&note).execute(conn)?;
    notes::table.load::<Note>(conn)?;
    Ok::<_, diesel::result::Error>(())
})?;
```

**SeaORM**: the transaction implements its connection trait, so entity
operations take `&txn`.

```rust
let txn = db.begin().await?;
note.insert(&txn).await?;
Note::find().all(&txn).await?;
txn.commit().await?;
```

## A join

**rupa**: declared relations, a typed join, and merged rows
([example](../crates/rupa/examples/sqlite_joins.rs)).

```rust
let joined: Vec<(Author, Option<Book>)> = select::<Author>().join::<Book>().left().all().execute(&pool)?;
```

**Diesel**: `joinable!` and `left_join`, with a nullable tuple.

```rust
let joined: Vec<(Author, Option<Book>)> = authors::table
    .left_join(books::table)
    .load(&mut conn)?;
```

**SeaORM**: relations on the entity, loaded with `find_with_related` or a
loader.

```rust
let joined: Vec<(author::Model, Vec<book::Model>)> =
    Author::find().find_with_related(Book).all(&db).await?;
```

## A JSON filter

**rupa**: a JSON column is a field of any serde type; its path is typed in the
query and bound, never spliced ([example](../crates/rupa/examples/sqlite_dsl.rs)).

```rust
select::<Profile>().filter(Profile::SETTINGS.path("theme").text().eq("dark")).all()
```

**Diesel**: JSON operators come from the backend's extension crates or `sql`
fragments. **SeaORM**: through SeaQuery expressions (`Expr::cust` or the
backend's JSON functions).

## Testing without a database

**rupa**: the in-memory backend evaluates the query IR with Postgres
semantics, so a repository runs unchanged in unit tests
([example](../crates/rupa/examples/memory_repository_test/main.rs)).

```rust
let mut db = MemoryDb::new();
db.register::<Account>();
let signups = Signups { accounts: Arc::new(AccountsRepo::new(db)) };
```

**Diesel**: tests usually run against a real database (SQLite in memory, or a
test transaction with `begin_test_transaction`). **SeaORM**: `MockDatabase`
returns scripted results and records the statements it was given.

## What each does better

- **Diesel** checks queries against a schema generated from the live database
  (`diesel print-schema`), so a renamed column fails to compile. It is the
  most mature of the three, with the largest ecosystem; async needs the
  separate `diesel-async`.
- **SeaORM** generates entities from an existing database, handles relations
  and eager loading in depth, saves nested object graphs in one call, and has a
  mature migration CLI and admin tooling.
- **rupa** declares repositories instead of writing them, runs the same
  repository on sync and async connections, pools and transactions, and tests
  it on an in-memory backend with real query semantics. Its schema is the Rust
  entity, not the database: drift is caught by migrations and tests, not by
  the compiler.
