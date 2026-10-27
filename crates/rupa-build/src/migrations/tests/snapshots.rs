use super::*;

#[test]
fn snapshot_files_and_the_snapshots_directory_generate_snapshots() {
    let fixture = Fixture::new(&[
        ("1_create.sql", "CREATE TABLE notes (title text);\n"),
        (
            "2_index.sql",
            "CREATE INDEX notes_title ON notes (title);\n",
        ),
        (
            "2_index.snapshot.sql",
            "-- Notes with their index.\nCREATE TABLE notes (title text);\nCREATE INDEX notes_title ON notes (title);\n",
        ),
        ("snapshots/3_seed.rs", "pub struct Seed;\n"),
    ]);

    let artifacts = fixture
        .discover(VersionScheme::Numbered)
        .expect("valid directory");
    let source = Migrations::new("primary")
        .generate(fixture.path())
        .expect("generated catalog");
    let order: Vec<_> = artifacts
        .iter()
        .map(|artifact| (artifact.version.to_string(), artifact.snapshot))
        .collect();

    assert_eq!(
        order,
        [
            ("1".to_owned(), false),
            ("2".to_owned(), false),
            ("2".to_owned(), true),
            ("3".to_owned(), true),
        ],
        "a snapshot follows the incremental of its version"
    );
    assert_eq!(
        source.matches("MigrationKind::Snapshot").count(),
        2,
        "{source}"
    );

    for expected in [
        "pub mod s2_index {",
        "S2Index(__rupa_migrate::Planned<s2_index::Index>)",
        "V2Index(__rupa_migrate::Planned<m2_index::Index>)",
        "S3Seed(s3_seed::Seed)",
    ] {
        assert!(
            source.contains(expected),
            "missing {expected:?} in:\n{source}"
        );
    }
}

#[test]
fn snapshots_without_down_scripts_or_shared_versions_are_required() {
    let cases: [RejectedCase; 5] = [
        (
            &[
                ("1_create.sql", "SELECT 1;"),
                ("1_create.snapshot.sql", "SELECT 1;"),
                ("1_create.snapshot.down.sql", "SELECT 2;"),
            ],
            |e| matches!(e, Error::SnapshotDown(_)),
        ),
        (
            &[
                ("1_create.sql", "SELECT 1;"),
                ("snapshots/1_create.sql", "SELECT 1;"),
                ("snapshots/1_create.down.sql", "SELECT 2;"),
            ],
            |e| matches!(e, Error::SnapshotDown(_)),
        ),
        (
            &[
                ("1_create.sql", "SELECT 1;"),
                ("1_first.snapshot.sql", "SELECT 1;"),
                ("snapshots/1_second.sql", "SELECT 1;"),
            ],
            |e| matches!(e, Error::Duplicate { version, .. } if version == "1"),
        ),
        (
            &[
                ("1_create.sql", "SELECT 1;"),
                ("snapshots/1_create.snapshot.sql", "SELECT 1;"),
            ],
            |e| matches!(e, Error::Name(_)),
        ),
        (
            &[
                ("1_create.sql", "SELECT 1;"),
                (
                    "snapshots/1_seed.rs",
                    "pub struct Seed;\nimpl<S> Migration<S> for Seed {\n    fn up(&self) {}\n    fn down(&self) {}\n}\n",
                ),
            ],
            |e| matches!(e, Error::SnapshotDown(path) if path.ends_with("snapshots/1_seed.rs")),
        ),
    ];

    for (files, expected) in cases {
        let fixture = Fixture::new(files);

        let error = fixture
            .discover(VersionScheme::Numbered)
            .expect_err("rejected snapshot");

        assert!(expected(&error), "{files:?}: {error}");
    }
}
