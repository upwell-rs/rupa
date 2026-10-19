use super::{Error, Violation};
use crate::migrations::isolation::{Layout, check};

fn helper(relative: &[&str]) -> Vec<String> {
    relative.iter().map(ToString::to_string).collect()
}

fn violation(source: &str, layout: Layout<'_>) -> Option<Violation> {
    match check("migration.rs", source, layout) {
        Ok(()) => None,
        Err(Error::Isolation { violation, .. }) => Some(violation),
        Err(error) => panic!("unexpected error: {error}"),
    }
}

#[test]
fn self_contained_migrations_and_external_crates_are_allowed() {
    let root = helper(&["mod.rs"]);
    let nested = helper(&["slug.rs"]);
    let deeper = helper(&["text", "slug.rs"]);
    let cases = [
        (
            "use rupa::{raw, select};\n#[derive(rupa::Entity)]\nstruct Note { id: i64 }\npub(crate) struct M;\nmod inner { use super::Note; }",
            Layout::File,
        ),
        (
            "mod slug;\nconst WORDS: &str = include_str!(\"data/words.txt\");",
            Layout::Directory {
                depth: 0,
                relative: &root,
            },
        ),
        (
            "use super::Note;",
            Layout::Directory {
                depth: 1,
                relative: &nested,
            },
        ),
        (
            "const W: &str = include_str!(\"../data.txt\");",
            Layout::Directory {
                depth: 2,
                relative: &deeper,
            },
        ),
        (
            "fn f() { let v = vec![self::g()]; }\nfn g() {}",
            Layout::File,
        ),
    ];

    for (source, layout) in cases {
        assert_eq!(violation(source, layout), None, "{source}");
    }
}

#[test]
fn reaching_application_code_or_foreign_files_is_rejected() {
    let nested = helper(&["slug.rs"]);
    let cases = [
        (
            "use crate::Note;",
            Layout::File,
            Violation::CratePath("crate::Note".into()),
        ),
        (
            "use crate::{Note, Repo};",
            Layout::File,
            Violation::CratePath("crate::Note".into()),
        ),
        (
            "fn f() { crate::db::Note::default(); }",
            Layout::File,
            Violation::CratePath("crate::db::Note::default".into()),
        ),
        (
            "use super::Note;",
            Layout::File,
            Violation::SuperPath("super::Note".into()),
        ),
        (
            "use super::super::Note;",
            Layout::Directory {
                depth: 1,
                relative: &nested,
            },
            Violation::SuperPath("super::super::Note".into()),
        ),
        (
            "fn f() { let n = vec![crate::Note]; }",
            Layout::File,
            Violation::CratePath("crate::Note".into()),
        ),
        (
            "macro_rules! m { () => { $crate::Note } }",
            Layout::File,
            Violation::CratePath("crate::Note".into()),
        ),
        (
            "#[path = \"../shared.rs\"]\nmod shared {}",
            Layout::File,
            Violation::PathAttribute,
        ),
        (
            "mod helpers;",
            Layout::File,
            Violation::OutOfLineModule("helpers".into()),
        ),
        (
            "const S: &str = include_str!(\"0001_x.sql\");",
            Layout::File,
            Violation::Include("0001_x.sql".into()),
        ),
        (
            "const S: &str = include_str!(\"../../outside.txt\");",
            Layout::Directory {
                depth: 1,
                relative: &nested,
            },
            Violation::Include("../../outside.txt".into()),
        ),
    ];

    for (source, layout, expected) in cases {
        assert_eq!(violation(source, layout), Some(expected), "{source}");
    }
}

#[test]
fn unparsable_sources_report_their_logical_path() {
    let error = check("2_broken.rs", "fn {", Layout::File).expect_err("parse failure");

    assert!(
        matches!(error, Error::Parse { ref path, .. } if path == "2_broken.rs"),
        "{error}"
    );
}
