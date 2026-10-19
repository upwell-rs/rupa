use rupa::migrate::schema::{Index, SchemaOp, Table};
use rupa::migrate::{
    Context, Error, Metadata, Migration, MigrationInfo, Planned, RunError, Session,
};

use super::fixture::{Author, Plans, Post, add_pinned, body, create_tables, planned};
use super::{MigrationHarness, catalog};

pub(super) fn case_planned_schema_creates_entity_tables<H: MigrationHarness>(h: &mut H) {
    let entries = [
        Planned(Plans(planned("1"), create_tables)),
        Planned(Plans(planned("2"), add_pinned)),
    ];

    h.connect()
        .apply(&catalog(&entries))
        .expect("schema plans apply");

    assert_eq!(
        h.count("SELECT count(*) FROM posts JOIN authors ON authors.id = posts.author_id WHERE title = 'hello' AND NOT pinned AND body IS NULL"),
        1
    );
}

pub(super) fn case_changed_plan_is_reported_as_changed_history<H: MigrationHarness>(h: &mut H) {
    let original = [Planned(Plans(planned("1"), create_tables))];
    let changed = [Planned(Plans(planned("1"), || {
        create_tables().create_index(Index::on("posts_body", Post::BODY))
    }))];

    let mut session = h.connect();
    session.apply(&catalog(&original)).expect("initial apply");

    assert!(matches!(
        session.apply(&catalog(&changed)),
        Err(RunError::Infrastructure {
            source: Error::History(message),
            ..
        }) if message.contains("changed checksum")
    ));
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

pub(super) fn case_scripted_migrations_run_schema_operations<H: MigrationHarness>(h: &mut H) {
    let entries = [Scripted(body("1", "scripted schema").0)];

    h.connect()
        .apply(&catalog(&entries))
        .expect("scripted schema apply");

    assert_eq!(
        h.count("SELECT count(*) FROM authors WHERE name = 'grace'"),
        1
    );
}

pub(super) fn case_ordinary_renames_and_indexes_work<H: MigrationHarness>(h: &mut H) {
    let entries = [body(
        "1",
        "CREATE TABLE notes(old_name TEXT); ALTER TABLE notes RENAME COLUMN old_name TO title; ALTER TABLE notes RENAME TO renamed; INSERT INTO renamed VALUES ('ok'); CREATE UNIQUE INDEX renamed_title ON renamed(title); DROP INDEX renamed_title;",
    )];

    h.connect()
        .apply(&catalog(&entries))
        .expect("ordinary DDL must remain supported");

    assert_eq!(
        h.count("SELECT count(*) FROM renamed WHERE title = 'ok'"),
        1
    );
}
