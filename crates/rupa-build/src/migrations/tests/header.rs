use crate::migrations::header::{Split, split};

#[test]
fn leading_comment_lines_become_the_description_and_leave_the_body_intact() {
    let source = "-- Create notes\n--   with a seed row  \nSELECT 1; -- trailing\n";

    let parsed = split(source, "--");

    assert_eq!(
        parsed,
        Split {
            description: Some("Create notes with a seed row".into()),
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
