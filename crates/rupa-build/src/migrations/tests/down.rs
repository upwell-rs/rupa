use super::discover::Body;
use super::*;

#[test]
fn down_files_pair_with_their_sql_migration() {
    let fixture = Fixture::new(&[
        ("1_create.sql", "CREATE TABLE notes (title text);\n"),
        ("1_create.down.sql", "-- Drops notes.\nDROP TABLE notes;\n"),
        (
            "2_index.sql",
            "CREATE INDEX notes_title ON notes (title);\n",
        ),
    ]);

    let artifacts = fixture
        .discover(VersionScheme::Numbered)
        .expect("valid directory");
    let source = Migrations::new("primary")
        .generate(fixture.path())
        .expect("generated catalog");

    assert!(
        matches!(
            &artifacts[0].body,
            Body::Sql { down: Some(down), .. }
                if down.path == "1_create.down.sql" && down.contents == "DROP TABLE notes;\n"
        ),
        "the down header is excluded like an up header: {:?}",
        artifacts[0]
    );
    assert!(matches!(&artifacts[1].body, Body::Sql { down: None, .. }));

    for expected in [
        "const DOWN: &str = \"DROP TABLE notes;\\n\";",
        "ArtifactInput::new(\"1_create.down.sql\", DOWN)",
        "::std::option::Option::Some(super::__rupa_migrate::Plan::new().sql(DOWN))",
    ] {
        assert!(
            source.contains(expected),
            "missing {expected:?} in:\n{source}"
        );
    }

    assert_eq!(source.matches("fn down(&self)").count(), 1, "{source}");
}

#[test]
fn down_files_without_a_matching_sql_migration_fail() {
    let cases: [RejectedCase; 5] = [
        (
            &[
                ("1_create.sql", "SELECT 1;"),
                ("2_drop.down.sql", "SELECT 2;"),
            ],
            |e| matches!(e, Error::UnpairedDown(path) if path.ends_with("2_drop.down.sql")),
        ),
        (
            &[
                ("1_create.sql", "SELECT 1;"),
                ("1_other.down.sql", "SELECT 2;"),
            ],
            |e| matches!(e, Error::UnpairedDown(_)),
        ),
        (
            &[
                ("1_backfill.rs", "pub struct Backfill;\n"),
                ("1_backfill.down.sql", "SELECT 2;"),
            ],
            |e| matches!(e, Error::RustDown(_)),
        ),
        (
            &[
                ("1_create.sql", "SELECT 1;"),
                (
                    "1_create.down.sql",
                    "-- rupa: { policy: nontransactional }\nSELECT 2;",
                ),
            ],
            |e| matches!(e, Error::DownFrontmatter(_)),
        ),
        (
            &[
                ("1_create.sql", "SELECT 1;"),
                ("1_create.down.sql", "SELECT 2;"),
                ("001_create.down.sql", "SELECT 3;"),
            ],
            |e| matches!(e, Error::Duplicate { version, .. } if version == "1"),
        ),
    ];

    for (files, expected) in cases {
        let fixture = Fixture::new(files);

        let error = fixture
            .discover(VersionScheme::Numbered)
            .expect_err("rejected down file");

        assert!(expected(&error), "{files:?}: {error}");
    }
}

#[test]
fn an_imperative_down_body_declares_the_migration_reversible() {
    let fixture = Fixture::new(&[
        (
            "1_backfill.rs",
            "pub struct Backfill;\nimpl<S> Migration<S> for Backfill {\n    fn up(&self) {}\n    fn down(&self) {}\n}\n",
        ),
        (
            "2_seed.rs",
            "pub struct Seed;\nimpl<S> Migration<S> for Seed {\n    fn up(&self) {}\n}\n",
        ),
    ]);

    let source = Migrations::new("primary")
        .generate(fixture.path())
        .expect("generated catalog");

    assert_eq!(
        source
            .matches("__rupa_migrate::Reversal::Imperative")
            .count(),
        1,
        "only the migration defining `down` is reversible:\n{source}"
    );
}
