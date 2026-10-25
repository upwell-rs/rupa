use rupa::migrate::schema::{ColumnDef, ColumnType};
use rupa::migrate::sync::Session;
use rupa::migrate::{Catalog, Error, Planned, RunError, ValidationError, VersionScheme};
use rupa_conformance::migrations::fixture::{Plans, Post, add_pinned, create_tables, planned};
use rupa_driver_rusqlite::SqliteExecutor;

use super::memory_runner;

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
        ["posts_author"]
    );
    assert_eq!(
        rows(&mut runner, "SELECT title || ':' || pinned FROM posts"),
        ["hello:0"]
    );
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
