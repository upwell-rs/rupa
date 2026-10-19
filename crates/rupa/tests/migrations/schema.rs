use rupa::migrate::schema::{ColumnDef, ColumnDefault, ColumnType, Index, SchemaOp, Table};
use rupa::migrate::{Planned, RunError, ValidationError};

use super::*;

/// The author shape the schema migrations below create.
#[derive(Debug, rupa::Entity)]
#[entity(table = "authors")]
struct Author {
    #[id(generated)]
    id: i64,
    name: String,
}

/// A post referencing its author.
#[derive(Debug, rupa::Entity)]
#[entity(table = "posts")]
struct Post {
    #[id(generated)]
    id: i64,
    #[column(references = Author)]
    author_id: i64,
    title: String,
    body: Option<String>,
}

fn create_tables() -> Plan {
    Plan::new()
        .create_table(Table::of::<Author>())
        .create_table(Table::of::<Post>())
        .create_index(Index::on("posts_title", Post::TITLE))
}

fn add_pinned() -> Plan {
    Plan::new()
        .add_column_to(
            <Post as rupa::Entity>::TABLE,
            ColumnDef::new("pinned", ColumnType::Bool)
                .default(ColumnDefault::Value(rupa::core::Value::Bool(false))),
        )
        .sql("INSERT INTO authors (name) VALUES ('ada'); INSERT INTO posts (author_id, title) VALUES (1, 'hello');")
}

fn rows(runner: &mut SqliteExecutor, sql: &str) -> Vec<String> {
    let connection = runner.connection();
    let mut statement = connection.prepare(sql).expect("valid inspection query");

    statement
        .query_map([], |row| row.get::<_, String>(0))
        .expect("inspection rows")
        .collect::<Result<_, _>>()
        .expect("text column")
}

#[test]
fn planned_schema_migrations_create_entity_tables_keys_and_indexes() {
    let entries = [
        Planned(Plans(planned("1"), create_tables)),
        Planned(Plans(planned("2"), add_pinned)),
    ];
    let catalog =
        Catalog::new("primary", VersionScheme::Numbered, &entries).expect("valid catalog");
    let mut runner = memory_runner();

    runner.apply(&catalog).expect("schema plans apply");

    assert_eq!(
        rows(
            &mut runner,
            "SELECT name FROM pragma_table_info('posts') ORDER BY cid"
        ),
        ["id", "author_id", "title", "body", "pinned"]
    );
    assert_eq!(
        rows(
            &mut runner,
            "SELECT \"table\" || '.' || \"to\" FROM pragma_foreign_key_list('posts')"
        ),
        ["authors.id"]
    );
    assert_eq!(
        rows(
            &mut runner,
            "SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'posts' AND sql IS NOT NULL"
        ),
        ["posts_title"]
    );
    assert_eq!(
        rows(&mut runner, "SELECT title || ':' || pinned FROM posts"),
        ["hello:0"]
    );
}

#[test]
fn a_changed_plan_is_reported_as_changed_history() {
    let original = [Planned(Plans(planned("1"), create_tables))];
    let changed = [Planned(Plans(planned("1"), || {
        create_tables().create_index(Index::on("posts_body", Post::BODY))
    }))];
    let original = Catalog::new("primary", VersionScheme::Numbered, &original).expect("original");
    let changed = Catalog::new("primary", VersionScheme::Numbered, &changed).expect("changed");
    let mut runner = memory_runner();

    runner.apply(&original).expect("initial apply");

    assert!(matches!(
        runner.apply(&changed),
        Err(RunError::Infrastructure {
            source: Error::History(message),
            ..
        }) if message.contains("changed checksum")
    ));
}

#[test]
fn schema_operations_the_dialect_cannot_express_fail_before_bootstrap() {
    let entries = [Planned(Plans(planned("1"), || {
        create_tables().add_column_to(
            <Post as rupa::Entity>::TABLE,
            ColumnDef::new("slug", ColumnType::Text),
        )
    }))];
    let catalog =
        Catalog::new("primary", VersionScheme::Numbered, &entries).expect("valid catalog");
    let mut runner = memory_runner();

    assert!(matches!(
        runner.apply(&catalog),
        Err(RunError::Infrastructure {
            source: Error::Validation(ValidationError::Schema(_)),
            ..
        })
    ));
    assert!(rows(&mut runner, "SELECT name FROM sqlite_master").is_empty());
}

/// A scripted migration mixing schema operations with arbitrary steps.
struct Scripted(&'static Metadata);

impl MigrationInfo for Scripted {
    fn info(&self) -> &'static Metadata {
        self.0
    }
}

impl<S: Session> Migration<S> for Scripted {
    type Error = Error<S::Error>;

    fn up(&self, context: &mut Context<'_, S>) -> Result<(), Self::Error> {
        context.schema(&SchemaOp::CreateTable(Table::of::<Author>()))?;
        context.script("INSERT INTO authors (name) VALUES ('grace')")
    }
}

#[test]
fn scripted_migrations_run_schema_operations_through_their_context() {
    let entries = [Scripted(body("1", "scripted schema").0)];
    let catalog =
        Catalog::new("primary", VersionScheme::Numbered, &entries).expect("valid catalog");
    let mut runner = memory_runner();

    runner.apply(&catalog).expect("scripted schema apply");

    assert_eq!(rows(&mut runner, "SELECT name FROM authors"), ["grace"]);
}
