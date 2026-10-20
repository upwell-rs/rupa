use rupa::drivers::sqlite::SqliteExecutor;
use rupa::migrate::MigrationInfo;
use rupa::migrate::sync::Session;
use rupa_example_db::migrations::{ENTRIES, catalog};

#[test]
fn discovered_catalog_applies_sql_single_file_and_directory_migrations_in_order() {
    let catalog = catalog().expect("generated catalog validates");
    let mut connection = SqliteExecutor::open_in_memory().expect("SQLite connection");

    let report = connection
        .apply(&catalog)
        .expect("apply discovered migrations");

    assert_eq!(
        report
            .applied
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["1", "2", "3", "4"]
    );

    let (title, slug): (String, String) = connection
        .connection()
        .query_row("SELECT title, slug FROM notes WHERE id = 1", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .expect("migrated note");

    assert_eq!(title, "HELLO; WORLD");
    assert_eq!(slug, "hello-world");

    let tagged: String = connection
        .connection()
        .query_row(
            "SELECT \"table\" || '.' || \"to\" FROM pragma_foreign_key_list('tags')",
            [],
            |row| row.get(0),
        )
        .expect("tags reference notes");

    assert_eq!(tagged, "notes.id");
}

#[test]
fn descriptions_come_from_file_headers_and_inputs_exclude_them() {
    let infos: Vec<_> = ENTRIES.iter().map(MigrationInfo::info).collect();

    assert_eq!(
        infos.iter().map(|i| i.description).collect::<Vec<_>>(),
        [
            "Create the notes table with a seed row.",
            "Trim and uppercase note titles in Rust.",
            "Add a slug column derived from each title.",
            "Create the tags table from its entity and index note slugs.",
        ]
    );
    assert!(
        infos[3].inputs.is_empty(),
        "a planned migration is identified by its plan, not its source"
    );
    assert_eq!(
        infos[2].inputs.iter().map(|i| i.path).collect::<Vec<_>>(),
        ["0003_add_slugs/mod.rs", "0003_add_slugs/slug.rs"]
    );
    assert!(
        infos
            .iter()
            .flat_map(|i| i.inputs)
            .all(|input| !input.contents.starts_with("--") && !input.contents.starts_with("//!")),
        "description headers are not part of the checksummed inputs"
    );
}
