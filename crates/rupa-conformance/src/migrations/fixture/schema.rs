//! The author and post tables, and the plans that create and change them.

use rupa::VarChar;
use rupa::migrate::Plan;
use rupa::migrate::schema::{ColumnDef, ColumnDefault, ColumnType, Gin, Index, IndexKind, Table};

/// The author shape the schema migrations below create.
#[derive(Debug, rupa::Entity)]
#[entity(table = "authors")]
pub struct Author {
    #[id(generated)]
    pub id: i64,
    pub name: String,
}

/// A post referencing its author.
#[derive(Debug, rupa::Entity)]
#[entity(table = "posts")]
pub struct Post {
    #[id(generated)]
    pub id: i64,
    #[column(references = Author)]
    pub author_id: i64,
    pub title: String,
    pub body: Option<String>,
    pub slug: Option<VarChar<64>>,
    pub meta: Option<serde_json::Value>,
}

/// Creates the author and post tables with an author index and a unique
/// index over the bounded slug, which every dialect can build.
pub fn create_tables() -> Plan {
    Plan::new()
        .create_table(Table::of::<Author>())
        .create_table(Table::of::<Post>())
        .create_index(Index::on("posts_author", Post::AUTHOR_ID))
        .create_index(Index::on("posts_slug", Post::SLUG).unique())
}

/// [`create_tables`] with an index over the unbounded `title` text, which
/// PostgreSQL and SQLite build and MySQL refuses.
pub fn create_tables_with_text_index() -> Plan {
    create_tables().create_index(Index::on("posts_title", Post::TITLE))
}

/// [`create_tables`] with a GIN index over the JSON `meta` column, which
/// only PostgreSQL builds.
pub fn create_tables_with_gin_index() -> Plan {
    create_tables().create_index(Gin::on("posts_meta", Post::META))
}

/// Adds a defaulted column to posts, then inserts a post.
pub fn add_pinned() -> Plan {
    Plan::new()
        .add_column_to(
            <Post as rupa::Entity>::TABLE,
            ColumnDef::new("pinned", ColumnType::Bool)
                .default(ColumnDefault::Value(rupa::Value::Bool(false))),
        )
        .sql("INSERT INTO authors (name) VALUES ('ada'); INSERT INTO posts (author_id, title) VALUES (1, 'hello');")
}
