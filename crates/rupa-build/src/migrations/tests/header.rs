use std::path::Path;

use rupa_migrate::Policy;

use crate::migrations::header::{Split, split};

#[test]
fn leading_comment_lines_become_the_description_and_leave_the_body_intact() {
    let source = "-- Create notes\n--   with a seed row  \nSELECT 1; -- trailing\n";

    let parsed = split(source, "--");

    assert_eq!(
        parsed,
        Split {
            description: Some("Create notes with a seed row".into()),
            directives: Vec::new(),
            body: "SELECT 1; -- trailing\n",
        }
    );
}

#[test]
fn bom_crlf_and_missing_headers_are_handled() {
    let cases = [
        ("\u{feff}//! Doc\r\nfn up() {}", Some("Doc"), "fn up() {}"),
        ("SELECT 1;", None, "SELECT 1;"),
        ("-- only a header", Some("only a header"), ""),
    ];

    for (source, description, body) in cases {
        let marker = if source.contains("//!") { "//!" } else { "--" };

        let parsed = split(source, marker);

        assert_eq!(parsed.description.as_deref(), description, "{source:?}");
        assert_eq!(parsed.body, body, "{source:?}");
    }
}

#[test]
fn editing_only_the_description_keeps_the_hashed_body() {
    let before = split("-- old wording\nSELECT 1;\n", "--");
    let after = split("-- new, better wording\nSELECT 1;\n", "--");

    assert_ne!(before.description, after.description);
    assert_eq!(before.body, after.body);
}

#[test]
fn directive_lines_declare_the_policy_and_stay_out_of_the_description() {
    let source = "-- Index notes without blocking writes.\n--  rupa:  nontransactional \nCREATE INDEX CONCURRENTLY i ON notes (title);\n";

    let header = split(source, "--")
        .header(Path::new("2_index.sql"))
        .expect("known directive");
    let default = split("//! Plain\nfn up() {}", "//!")
        .header(Path::new("1_plain.rs"))
        .expect("no directives");

    assert_eq!(
        header.description.as_deref(),
        Some("Index notes without blocking writes.")
    );
    assert_eq!(header.settings.policy, Policy::NonTransactional);
    assert_eq!(default.settings.policy, Policy::TransactionalRequired);
}
