use rupa::DialectId;
use rupa::migrate::schema::Index;
use rupa::migrate::{Blocker, Error, Planned, RunError, ValidationError};
use rupa::sql::{IndexRefusal, RenderError, SchemaError};

use super::support::{dialect, planned, script};
use super::{Harness, Session, catalog};
use crate::migrations::fixture::{
    Plans, Post, Scripted, add_pinned, create_tables, create_tables_with_gin_index,
    create_tables_with_text_index,
};

pub(super) fn case_planned_schema_creates_entity_tables<H: Harness>(h: &mut H) {
    let entries = [
        Planned(Plans(planned::<H>("1"), create_tables)),
        Planned(Plans(planned::<H>("2"), add_pinned)),
    ];

    wait!(h, h.connect().apply(&catalog(&entries))).expect("schema plans apply");

    assert_eq!(
        h.count("SELECT count(*) FROM posts JOIN authors ON authors.id = posts.author_id WHERE title = 'hello' AND NOT pinned AND body IS NULL"),
        1
    );
}

/// PostgreSQL and SQLite index unbounded text. MySQL cannot, and says so at
/// preflight: nothing is acquired or created, so the target stays unmanaged.
pub(super) fn case_text_indexes_work_or_fail_typed_before_acquisition<H: Harness>(h: &mut H) {
    let entries = [Planned(Plans(
        planned::<H>("1"),
        create_tables_with_text_index,
    ))];
    let result = wait!(h, h.connect().apply(&catalog(&entries)));

    if dialect(h) != DialectId::MySql {
        result.expect("unbounded text indexes where the dialect supports them");

        assert_eq!(h.count("SELECT count(*) FROM posts"), 0);

        return;
    }

    assert!(
        matches!(
            result,
            Err(RunError::Infrastructure {
                source: Error::Validation(ValidationError::Schema(ref error)),
                ..
            }) if matches!(
                **error,
                RenderError::Schema(ref schema) if matches!(
                    **schema,
                    SchemaError::Index {
                        index: "posts_title",
                        dialect: DialectId::MySql,
                        reason: IndexRefusal::Unbounded { column: "title", .. },
                    }
                )
            )
        ),
        "{result:?}"
    );
    assert!(!h.table_exists("_rupa_migrations"));
    assert!(!h.table_exists("posts"));
}

/// GIN over a JSON column builds on PostgreSQL. Other dialects refuse the
/// kind at preflight, before anything is created.
pub(super) fn case_gin_indexes_build_on_postgres_and_fail_typed_elsewhere<H: Harness>(h: &mut H) {
    let entries = [Planned(Plans(
        planned::<H>("1"),
        create_tables_with_gin_index,
    ))];
    let result = wait!(h, h.connect().apply(&catalog(&entries)));

    if dialect(h) == DialectId::Postgres {
        result.expect("GIN over a JSON column on PostgreSQL");

        assert_eq!(h.count("SELECT count(*) FROM posts"), 0);

        return;
    }

    assert!(
        matches!(
            result,
            Err(RunError::Infrastructure {
                source: Error::Validation(ValidationError::Schema(ref error)),
                ..
            }) if matches!(
                **error,
                RenderError::Schema(ref schema) if matches!(
                    **schema,
                    SchemaError::Index {
                        index: "posts_meta",
                        reason: IndexRefusal::Kind { kind: "gin", .. },
                        ..
                    }
                )
            )
        ),
        "{result:?}"
    );
    assert!(!h.table_exists("_rupa_migrations"));
    assert!(!h.table_exists("posts"));
}

pub(super) fn case_changed_plan_is_reported_as_changed_history<H: Harness>(h: &mut H) {
    let original = [Planned(Plans(planned::<H>("1"), create_tables))];
    let changed = [Planned(Plans(planned::<H>("1"), || {
        create_tables().create_index(Index::on("posts_author_again", Post::AUTHOR_ID).unique())
    }))];

    let mut session = h.connect();
    wait!(h, session.apply(&catalog(&original))).expect("initial apply");

    assert!(matches!(
        wait!(h, session.apply(&catalog(&changed))),
        Err(RunError::Infrastructure {
            source: Error::Blocked(blockers),
            ..
        }) if matches!(blockers.as_slice(), [Blocker::Changed { .. }])
    ));
}

pub(super) fn case_scripted_migrations_run_schema_operations<H: Harness>(h: &mut H) {
    let entries = [Scripted(script::<H>("1", "scripted schema").0)];

    wait!(h, h.connect().apply(&catalog(&entries))).expect("scripted schema apply");

    assert_eq!(
        h.count("SELECT count(*) FROM authors WHERE name = 'grace'"),
        1
    );
}

/// MySQL spells `DROP INDEX` and text indexes differently; its driver tests
/// cover the same DDL.
pub(super) fn case_ordinary_renames_and_indexes_work<H: Harness>(h: &mut H) {
    if dialect(h) == DialectId::MySql {
        return;
    }

    let entries = [script::<H>(
        "1",
        "CREATE TABLE notes(old_name TEXT); ALTER TABLE notes RENAME COLUMN old_name TO title; ALTER TABLE notes RENAME TO renamed; INSERT INTO renamed VALUES ('ok'); CREATE UNIQUE INDEX renamed_title ON renamed(title); DROP INDEX renamed_title;",
    )];

    wait!(h, h.connect().apply(&catalog(&entries))).expect("ordinary DDL must remain supported");

    assert_eq!(
        h.count("SELECT count(*) FROM renamed WHERE title = 'ok'"),
        1
    );
}
