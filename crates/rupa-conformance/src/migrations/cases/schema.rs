use rupa::migrate::schema::Index;
use rupa::migrate::{Error, Planned, RunError};

use super::{Harness, Session, catalog};
use crate::migrations::fixture::{Plans, Post, Scripted, add_pinned, body, create_tables, planned};

pub(super) fn case_planned_schema_creates_entity_tables<H: Harness>(h: &mut H) {
    let entries = [
        Planned(Plans(planned("1"), create_tables)),
        Planned(Plans(planned("2"), add_pinned)),
    ];

    wait!(h, h.connect().apply(&catalog(&entries))).expect("schema plans apply");

    assert_eq!(
        h.count("SELECT count(*) FROM posts JOIN authors ON authors.id = posts.author_id WHERE title = 'hello' AND NOT pinned AND body IS NULL"),
        1
    );
}

pub(super) fn case_changed_plan_is_reported_as_changed_history<H: Harness>(h: &mut H) {
    let original = [Planned(Plans(planned("1"), create_tables))];
    let changed = [Planned(Plans(planned("1"), || {
        create_tables().create_index(Index::on("posts_body", Post::BODY))
    }))];

    let mut session = h.connect();
    wait!(h, session.apply(&catalog(&original))).expect("initial apply");

    assert!(matches!(
        wait!(h, session.apply(&catalog(&changed))),
        Err(RunError::Infrastructure {
            source: Error::History(message),
            ..
        }) if message.contains("changed checksum")
    ));
}

pub(super) fn case_scripted_migrations_run_schema_operations<H: Harness>(h: &mut H) {
    let entries = [Scripted(body("1", "scripted schema").0)];

    wait!(h, h.connect().apply(&catalog(&entries))).expect("scripted schema apply");

    assert_eq!(
        h.count("SELECT count(*) FROM authors WHERE name = 'grace'"),
        1
    );
}

pub(super) fn case_ordinary_renames_and_indexes_work<H: Harness>(h: &mut H) {
    let entries = [body(
        "1",
        "CREATE TABLE notes(old_name TEXT); ALTER TABLE notes RENAME COLUMN old_name TO title; ALTER TABLE notes RENAME TO renamed; INSERT INTO renamed VALUES ('ok'); CREATE UNIQUE INDEX renamed_title ON renamed(title); DROP INDEX renamed_title;",
    )];

    wait!(h, h.connect().apply(&catalog(&entries))).expect("ordinary DDL must remain supported");

    assert_eq!(
        h.count("SELECT count(*) FROM renamed WHERE title = 'ok'"),
        1
    );
}
