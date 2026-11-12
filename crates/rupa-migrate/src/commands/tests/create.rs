use std::fs;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::super::create::create;
use super::*;

/// 2026-10-27 09:30:00 UTC.
const NOW: u64 = 1_793_093_400;

fn at(seconds: u64) -> SystemTime {
    UNIX_EPOCH + Duration::from_secs(seconds)
}

fn new<'a>(scratch: &'a Scratch, name: &'a str, authoring: Authoring) -> Create<'a> {
    Create {
        dir: &scratch.0,
        name,
        kind: MigrationKind::Incremental,
        authoring,
        down: false,
    }
}

fn names(created: &Created, scratch: &Scratch) -> Vec<String> {
    created
        .paths
        .iter()
        .map(|path| {
            path.strip_prefix(&scratch.0)
                .expect("written inside the directory")
                .display()
                .to_string()
        })
        .collect()
}

#[test]
fn a_numbered_incremental_follows_the_catalog_and_the_directory() {
    let scratch = Scratch::new("numbered");
    let entries = NUMBERED;
    let numbered = catalog(&entries);

    fs::write(scratch.0.join("0003_unbuilt.sql"), "").expect("an unbuilt migration");

    let created = create(
        &numbered,
        &Create {
            down: true,
            ..new(&scratch, "add_tags", Authoring::Sql)
        },
        at(NOW),
    )
    .expect("create");
    let body = fs::read_to_string(&created.paths[0]).expect("body");

    assert_eq!(created.version, version("4"), "after the unbuilt 3 as well");
    assert_eq!(
        names(&created, &scratch),
        ["0004_add_tags.sql", "0004_add_tags.down.sql"],
        "padded like the directory's names, the down script a sibling"
    );
    assert_eq!(
        body, "-- Add tags.\n",
        "the description is excluded from the checksum"
    );
}

#[test]
fn an_empty_numbered_stream_starts_at_one() {
    let scratch = Scratch::new("empty");
    let empty = catalog(&[]);
    let created = create(
        &empty,
        &new(&scratch, "create_notes", Authoring::Sql),
        at(NOW),
    )
    .expect("create");

    assert_eq!(names(&created, &scratch), ["0001_create_notes.sql"]);
}

#[test]
fn a_timestamped_incremental_takes_the_utc_time() {
    let scratch = Scratch::new("timestamp");
    let empty: [Entry; 0] = [];
    let timestamped =
        Catalog::new("primary", VersionScheme::UtcTimestamp, &empty).expect("valid catalog");
    let first = create(
        &timestamped,
        &new(&scratch, "create_notes", Authoring::Sql),
        at(NOW),
    )
    .expect("create");
    let leap = create(
        &timestamped,
        &new(&scratch, "leap", Authoring::Sql),
        at(1_709_251_199),
    )
    .expect("create on a leap day");
    let again = create(
        &timestamped,
        &new(&scratch, "again", Authoring::Sql),
        at(NOW),
    );

    assert_eq!(names(&first, &scratch), ["20261027093000_create_notes.sql"]);
    assert_eq!(names(&leap, &scratch), ["20240229235959_leap.sql"]);
    assert!(
        matches!(again, Err(CreateError::Taken(taken)) if taken == first.version),
        "the same second is taken: {again:?}"
    );
}

#[test]
fn a_snapshot_stands_for_the_newest_incremental() {
    let scratch = Scratch::new("snapshot");
    let entries = NUMBERED;
    let numbered = catalog(&entries);
    let snapshot = Create {
        kind: MigrationKind::Snapshot,
        ..new(&scratch, "notes", Authoring::Sql)
    };
    let created = create(&numbered, &snapshot, at(NOW)).expect("create");
    let again = create(&numbered, &snapshot, at(NOW));
    let reversible = create(
        &numbered,
        &Create {
            down: true,
            ..snapshot
        },
        at(NOW),
    );
    let nothing = create(&catalog(&[]), &snapshot, at(NOW));

    assert_eq!(created.version, version("2"));
    assert_eq!(names(&created, &scratch), ["snapshots/0002_notes.sql"]);
    assert!(matches!(again, Err(CreateError::Taken(_))), "{again:?}");
    assert!(matches!(reversible, Err(CreateError::SnapshotDown)));
    assert!(matches!(nothing, Err(CreateError::NoIncremental)));
}

#[test]
fn rust_sources_name_the_type_rupa_build_expects() {
    let scratch = Scratch::new("rust");
    let empty = catalog(&[]);
    let mut sources = Vec::new();

    for (name, authoring) in [
        ("plan_tags", Authoring::Planned),
        ("sync_tags", Authoring::Sync),
        ("async_tags", Authoring::Async),
    ] {
        let created = create(
            &empty,
            &Create {
                down: true,
                ..new(&scratch, name, authoring)
            },
            at(NOW),
        )
        .expect("create");

        assert_eq!(created.paths.len(), 1, "the down body is in the same file");
        sources.push(fs::read_to_string(&created.paths[0]).expect("source"));
    }

    assert!(sources[0].contains("impl PlannedMigration for PlanTags {"));
    assert!(sources[0].contains("fn down(&self) -> Option<Plan>"));
    assert!(sources[1].contains("use rupa::migrate::sync::{Context, Migration, Session};"));
    assert!(sources[1].contains("impl<S: Session> Migration<S> for SyncTags {"));
    assert!(sources[1].contains("    fn down(&self, context"));
    assert!(sources[2].contains("use rupa::migrate::asynchronous::"));
    assert!(sources[2].contains("    async fn up(&self, context"));
    assert!(sources.iter().all(|source| source.starts_with("//! ")));
}

#[test]
fn nothing_is_overwritten_or_left_half_written() {
    let scratch = Scratch::new("collision");
    let empty = catalog(&[]);
    let down = Create {
        down: true,
        ..new(&scratch, "create_notes", Authoring::Sql)
    };

    fs::write(scratch.0.join("0001_create_notes.down.sql"), "kept").expect("a stray down");

    let refused = create(&empty, &down, at(NOW));

    assert!(
        matches!(refused, Err(CreateError::Exists(_))),
        "{refused:?}"
    );
    assert!(
        !scratch.0.join("0001_create_notes.sql").exists(),
        "the body written first is removed again"
    );
    assert_eq!(
        fs::read_to_string(scratch.0.join("0001_create_notes.down.sql")).expect("stray"),
        "kept"
    );
}

#[test]
fn names_follow_rupa_build() {
    let scratch = Scratch::new("names");
    let empty = catalog(&[]);

    for name in ["Notes", "1notes", "add-tags", "self", ""] {
        let refused = create(&empty, &new(&scratch, name, Authoring::Sql), at(NOW));

        assert!(
            matches!(refused, Err(CreateError::Name(_))),
            "{name:?}: {refused:?}"
        );
    }
}
