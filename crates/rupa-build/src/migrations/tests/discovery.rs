use rupa_migrate::Policy;

use super::discover::Body;
use super::*;

fn versions(artifacts: &[discover::Artifact]) -> Vec<String> {
    artifacts.iter().map(|a| a.version.to_string()).collect()
}

#[test]
fn numbered_prefixes_are_canonicalized_and_sorted_numerically() {
    let fixture = Fixture::new(&[
        ("0010_later.sql", "SELECT 10;"),
        ("0002_add_index.sql", "SELECT 2;"),
        (".gitkeep", ""),
    ]);

    let artifacts = fixture
        .discover(VersionScheme::Numbered)
        .expect("valid directory");

    assert_eq!(versions(&artifacts), ["2", "10"]);
    assert_eq!(artifacts[0].description, "add index");
    assert!(!artifacts[0].documented);
}

#[test]
fn utc_timestamp_names_are_accepted() {
    let fixture = Fixture::new(&[("20261019120000_create.sql", "SELECT 1;")]);

    let artifacts = fixture
        .discover(VersionScheme::UtcTimestamp)
        .expect("valid timestamp");

    assert_eq!(versions(&artifacts), ["20261019120000"]);
}

#[test]
fn aliased_versions_collide() {
    let fixture = Fixture::new(&[
        ("1_first.sql", "SELECT 1;"),
        ("001_second.sql", "SELECT 2;"),
    ]);

    let error = fixture
        .discover(VersionScheme::Numbered)
        .expect_err("duplicate version");

    assert!(
        matches!(error, Error::Duplicate { ref version, .. } if version == "1"),
        "{error}"
    );
}

#[test]
fn malformed_entries_fail_instead_of_being_skipped() {
    let cases: [&[(&str, &str)]; 6] = [
        &[("README.md", "notes")],
        &[("create.sql", "SELECT 1;")],
        &[("1_Create.sql", "SELECT 1;")],
        &[("1_.sql", "SELECT 1;")],
        &[("1_helpers/lib.rs", "")],
        &[("0_zero.sql", "SELECT 0;")],
    ];

    for files in cases {
        let fixture = Fixture::new(files);

        let error = fixture
            .discover(VersionScheme::Numbered)
            .expect_err("malformed entry");

        assert!(
            matches!(error, Error::Name(_) | Error::Version { .. }),
            "{files:?}: {error}"
        );
    }
}

#[test]
fn names_and_inputs_the_runtime_would_reject_fail_the_build() {
    let cases: [RejectedCase; 6] = [
        (&[("1_self_.sql", "SELECT 1;")], |e| {
            matches!(e, Error::ReservedName(_))
        }),
        (&[("1_self__/mod.rs", "pub struct S;")], |e| {
            matches!(e, Error::ReservedName(_))
        }),
        (&[("1_self.sql", "SELECT 1;")], |e| {
            matches!(e, Error::ReservedName(_))
        }),
        (&[("1_self.rs", "pub struct S;")], |e| {
            matches!(e, Error::ReservedName(_))
        }),
        (
            &[
                ("2_data/mod.rs", "pub struct Data;"),
                ("2_data/we:ird.txt", "x"),
            ],
            |e| matches!(e, Error::InputPath(p) if p == "2_data/we:ird.txt"),
        ),
        (&[("3_lone_cr.sql", "SELECT 1;\rSELECT 2;")], |e| {
            matches!(e, Error::CarriageReturn(_))
        }),
    ];

    for (files, expected) in cases {
        let fixture = Fixture::new(files);

        let error = fixture
            .discover(VersionScheme::Numbered)
            .expect_err("rejected input");

        assert!(expected(&error), "{files:?}: {error}");
    }
}

#[test]
fn crlf_sources_remain_valid() {
    let fixture = Fixture::new(&[("1_crlf.sql", "-- Windows checkout\r\nSELECT 1;\r\n")]);

    let artifacts = fixture
        .discover(VersionScheme::Numbered)
        .expect("CRLF is normalized at runtime");

    assert_eq!(artifacts[0].description, "Windows checkout");
}

#[test]
fn an_empty_directory_is_rejected() {
    let fixture = Fixture::new(&[]);

    let error = fixture
        .discover(VersionScheme::Numbered)
        .expect_err("no migrations");

    assert!(matches!(error, Error::Empty(_)), "{error}");
}

#[cfg(unix)]
#[test]
fn symbolic_links_are_rejected() {
    let fixture = Fixture::new(&[("source.txt", "SELECT 1;")]);

    std::os::unix::fs::symlink(
        fixture.path().join("source.txt"),
        fixture.path().join("1_linked.sql"),
    )
    .expect("create symlink");
    std::fs::remove_file(fixture.path().join("source.txt")).expect("remove link target");

    let error = fixture
        .discover(VersionScheme::Numbered)
        .expect_err("symlink input");

    assert!(matches!(error, Error::Symlink(_)), "{error}");
}

#[test]
fn directory_migrations_hash_every_private_file_without_doc_headers() {
    let fixture = Fixture::new(&[
        (
            "3_add_slugs/mod.rs",
            "//! Add slugs.\nmod slug;\npub struct AddSlugs;\n",
        ),
        (
            "3_add_slugs/slug.rs",
            "//! Helper docs.\npub(super) fn slugify() {}\n",
        ),
        ("3_add_slugs/data/words.txt", "alpha\n"),
    ]);

    let artifacts = fixture
        .discover(VersionScheme::Numbered)
        .expect("directory migration");
    let Body::Rust { inputs, .. } = &artifacts[0].body else {
        panic!("expected a Rust migration");
    };
    let inputs: Vec<_> = inputs
        .iter()
        .map(|i| (i.path.as_str(), i.contents.as_str()))
        .collect();

    assert_eq!(artifacts[0].description, "Add slugs.");
    assert_eq!(
        inputs,
        [
            ("3_add_slugs/data/words.txt", "alpha\n"),
            ("3_add_slugs/mod.rs", "mod slug;\npub struct AddSlugs;\n"),
            ("3_add_slugs/slug.rs", "pub(super) fn slugify() {}\n"),
        ]
    );
}

#[test]
fn generated_catalog_names_types_after_files() {
    let fixture = Fixture::new(&[
        (
            "0001_create_notes.sql",
            "-- Create notes.\nCREATE TABLE notes (id INTEGER);\n",
        ),
        ("0002_normalize_titles.rs", "pub struct NormalizeTitles;\n"),
    ]);

    let source = Migrations::new("primary")
        .generate(fixture.path())
        .expect("generated catalog");

    for expected in [
        "pub const STREAM: &str = \"primary\";",
        "pub mod m1_create_notes {",
        "pub struct CreateNotes;",
        "const SQL: &str = \"CREATE TABLE notes (id INTEGER);\\n\";",
        "#[doc = \"normalize titles\"]",
        "pub mod m2_normalize_titles;",
        "V1CreateNotes(__rupa_migrate::Planned<m1_create_notes::CreateNotes>),",
        "V2NormalizeTitles(m2_normalize_titles::NormalizeTitles),",
    ] {
        assert!(
            source.contains(expected),
            "missing {expected:?} in:\n{source}"
        );
    }
}

#[test]
fn stream_names_must_be_safe_file_names() {
    let fixture = Fixture::new(&[("1_a.sql", "SELECT 1;")]);

    for stream in ["", "a/b", "has space", "../up"] {
        let error = Migrations::new(stream)
            .generate(fixture.path())
            .expect_err("unsafe stream");

        assert!(matches!(error, Error::Stream(_)), "{stream:?}: {error}");
    }
}

#[test]
fn planned_rust_migrations_are_identified_by_their_plan() {
    let fixture = Fixture::new(&[
        (
            "4_index_slugs.rs",
            "//! Index slugs.\nuse crate::Note;\npub struct IndexSlugs;\nimpl rupa::migrate::PlannedMigration for IndexSlugs { fn plan(&self) -> rupa::migrate::Plan { todo!() } }\n",
        ),
        (
            "5_tags/mod.rs",
            "use super::super::Tag;\nmod helper;\npub struct Tags;\nimpl PlannedMigration for Tags { fn plan(&self) -> Plan { helper::plan() } }\n",
        ),
        ("5_tags/helper.rs", "pub fn plan() {}\n"),
    ]);

    let artifacts = fixture
        .discover(VersionScheme::Numbered)
        .expect("planned migrations may use shared code");
    let source = Migrations::new("primary")
        .generate(fixture.path())
        .expect("generated catalog");

    for artifact in &artifacts {
        assert!(
            matches!(&artifact.body, Body::Rust { kind, inputs, .. } if kind.planned && inputs.is_empty()),
            "{artifact:?}"
        );
    }

    assert_eq!(artifacts[0].description, "Index slugs.");

    for expected in [
        "V4IndexSlugs(__rupa_migrate::Planned<m4_index_slugs::IndexSlugs>),",
        "Migrations::V5Tags(__rupa_migrate::Planned(m5_tags::Tags)),",
        "inputs: &[],",
    ] {
        assert!(
            source.contains(expected),
            "missing {expected:?} in:\n{source}"
        );
    }
}

#[test]
fn scripted_migrations_stay_isolated_and_kinds_do_not_mix() {
    let scripted = Fixture::new(&[(
        "1_scripted.rs",
        "use crate::Note;\npub struct Scripted;\nimpl<S> Migration<S> for Scripted {}\n",
    )]);
    let mixed = Fixture::new(&[(
        "1_mixed.rs",
        "pub struct Mixed;\nimpl PlannedMigration for Mixed {}\nimpl<S> Migration<S> for Mixed {}\n",
    )]);

    assert!(matches!(
        scripted.discover(VersionScheme::Numbered),
        Err(Error::Isolation { .. })
    ));
    assert!(matches!(
        mixed.discover(VersionScheme::Numbered),
        Err(Error::MixedKinds(path)) if path == "1_mixed.rs"
    ));
}

#[test]
fn frontmatter_sets_each_migrations_policy() {
    let fixture = Fixture::new(&[
        ("1_create.sql", "CREATE TABLE notes (title text);\n"),
        (
            "2_index.sql",
            "-- Index titles without blocking writes.\n-- rupa:\n--   policy: nontransactional\nCREATE INDEX CONCURRENTLY notes_title ON notes (title);\n",
        ),
        (
            "3_backfill.rs",
            "//! rupa: { policy: nontransactional }\npub struct Backfill;\n",
        ),
        (
            "4_split/mod.rs",
            "//! rupa:\n//!   policy: nontransactional\nmod helper;\npub struct Split;\n",
        ),
        ("4_split/helper.rs", "pub(super) fn step() {}\n"),
    ]);

    let artifacts = fixture
        .discover(VersionScheme::Numbered)
        .expect("valid directory");
    let source = Migrations::new("primary")
        .generate(fixture.path())
        .expect("generated catalog");

    assert_eq!(
        artifacts.iter().map(|a| a.policy).collect::<Vec<_>>(),
        [
            Policy::TransactionalRequired,
            Policy::NonTransactional,
            Policy::NonTransactional,
            Policy::NonTransactional,
        ]
    );
    assert_eq!(
        artifacts[1].description,
        "Index titles without blocking writes."
    );
    assert_eq!(
        source.matches("Policy::NonTransactional,").count(),
        3,
        "{source}"
    );
}

#[test]
fn frontmatter_must_be_known_and_declared_by_the_root() {
    let unknown = Fixture::new(&[("1_x.sql", "-- rupa:\n--   autocommit: true\nSELECT 1;\n")]);
    let helper = Fixture::new(&[
        ("1_split/mod.rs", "mod helper;\npub struct Split;\n"),
        (
            "1_split/helper.rs",
            "//! rupa: { policy: nontransactional }\npub(super) fn step() {}\n",
        ),
    ]);

    assert!(matches!(
        unknown.discover(VersionScheme::Numbered),
        Err(Error::Frontmatter { message, .. }) if message.contains("autocommit")
    ));
    assert!(matches!(
        helper.discover(VersionScheme::Numbered),
        Err(Error::HelperFrontmatter(path)) if path.ends_with("helper.rs")
    ));
}
