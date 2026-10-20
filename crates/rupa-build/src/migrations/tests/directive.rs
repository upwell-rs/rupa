use std::path::Path;

use rupa_migrate::Policy;

use crate::migrations::directive::{Directive, Settings};
use crate::migrations::header::split;
use crate::migrations::{DirectiveError, Error};

fn refusal(source: &str) -> (String, DirectiveError) {
    match split(source, "--").header(Path::new("1_x.sql")) {
        Err(Error::Directive {
            directive, reason, ..
        }) => (directive, reason),
        other => panic!("{source:?} must be refused, got {other:?}"),
    }
}

#[test]
fn keywords_parse_with_surrounding_whitespace_only() {
    assert_eq!(
        Directive::parse("nontransactional"),
        Ok(Directive::NonTransactional)
    );
    assert_eq!(
        Directive::parse("nontransactional = yes"),
        Err(DirectiveError::UnexpectedValue)
    );
    assert_eq!(
        Directive::parse("NonTransactional"),
        Err(DirectiveError::Unknown)
    );
}

#[test]
fn settings_start_from_defaults_and_apply_each_directive() {
    assert_eq!(
        Settings::from_directives(&[]).policy,
        Policy::TransactionalRequired
    );
    assert_eq!(
        Settings::from_directives(&[Directive::NonTransactional]).policy,
        Policy::NonTransactional
    );
}

#[test]
fn headers_refuse_unknown_valued_and_repeated_directives() {
    let cases = [
        (
            "-- rupa: autocommit\nSELECT 1;",
            "autocommit",
            DirectiveError::Unknown,
        ),
        (
            "-- rupa: nontransactional=true\nSELECT 1;",
            "nontransactional=true",
            DirectiveError::UnexpectedValue,
        ),
        (
            "-- rupa: nontransactional\n-- rupa: nontransactional\nSELECT 1;",
            "nontransactional",
            DirectiveError::Duplicate,
        ),
    ];

    for (source, directive, reason) in cases {
        assert_eq!(
            refusal(source),
            (directive.to_owned(), reason),
            "{source:?}"
        );
    }
}

#[test]
fn unknown_directives_list_every_known_keyword() {
    let error = split("-- rupa: autocommit\nSELECT 1;", "--")
        .header(Path::new("1_x.sql"))
        .expect_err("unknown directive");

    assert_eq!(
        error.to_string(),
        "1_x.sql: `rupa: autocommit`: unknown directive; known directives: nontransactional"
    );
}
