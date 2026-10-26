use rupa::drivers::sqlite::SqliteExecutor;
use rupa::migrate::sync::Session;
use rupa::migrate::{Blocker, Error, MigrationInfo, RevertTarget, RunError, Version};
use rupa_example_db::migrations::{ENTRIES, SCHEME, catalog};

fn version(text: &str) -> Version {
    SCHEME.parse(text).expect("version")
}

fn columns(connection: &mut SqliteExecutor, table: &str) -> Vec<String> {
    let mut statement = connection
        .connection()
        .prepare("SELECT name FROM pragma_table_info(?1)")
        .expect("column query");

    statement
        .query_map([table], |row| row.get(0))
        .expect("columns")
        .collect::<Result<_, _>>()
        .expect("column names")
}

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
    assert_eq!(
        infos[0].inputs.iter().map(|i| i.path).collect::<Vec<_>>(),
        ["0001_create_notes.sql", "0001_create_notes.down.sql"],
        "a SQL migration's down file is part of its artifact"
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

#[test]
fn sql_planned_and_imperative_down_bodies_revert_the_newest_installations() {
    let catalog = catalog().expect("generated catalog validates");
    let mut connection = SqliteExecutor::open_in_memory().expect("SQLite connection");

    connection
        .apply(&catalog)
        .expect("apply discovered migrations");

    let refused = connection.revert(&catalog, RevertTarget::Last(4));

    assert!(
        matches!(
            &refused,
            Err(RunError::Infrastructure { source: Error::Blocked(blockers), .. })
                if *blockers == [Blocker::Irreversible(version("2"))]
        ),
        "the irreversible Rust migration refuses the whole revert: {refused:?}"
    );

    let report = connection
        .revert(&catalog, RevertTarget::To(version("2")))
        .expect("revert the planned and directory migrations");

    assert_eq!(report.reverted, [version("4"), version("3")]);
    assert_eq!(report.remaining, [version("1"), version("2")]);
    assert!(
        columns(&mut connection, "tags").is_empty(),
        "the down plan drops tags"
    );
    assert_eq!(columns(&mut connection, "notes"), ["id", "title"]);

    let reapplied = connection
        .apply(&catalog)
        .expect("reapply the reverted migrations");

    assert_eq!(reapplied.applied, [version("3"), version("4")]);
}
