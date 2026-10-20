use std::path::Path;

use rupa_migrate::Policy;

use crate::migrations::Error;
use crate::migrations::frontmatter::Frontmatter;
use crate::migrations::header::split;

fn policy(source: &str) -> Result<Policy, Error> {
    let header = split(source, "--").header(Path::new("1_x.sql"))?;

    Ok(header.frontmatter.unwrap_or_default().policy)
}

#[test]
fn block_and_flow_forms_declare_the_same_settings() {
    let block = "-- rupa:\n--   policy: nontransactional\nSELECT 1;";
    let flow = "-- rupa: { policy: nontransactional }\nSELECT 1;";
    let explicit = "-- rupa:\n--   policy: transactional\nSELECT 1;";

    assert_eq!(policy(block).expect("block form"), Policy::NonTransactional);
    assert_eq!(policy(flow).expect("flow form"), Policy::NonTransactional);
    assert_eq!(
        policy(explicit).expect("explicit default"),
        Policy::TransactionalRequired
    );
}

#[test]
fn absent_or_empty_frontmatter_keeps_the_defaults() {
    assert_eq!(
        policy("-- Plain.\nSELECT 1;").expect("no frontmatter"),
        Policy::TransactionalRequired
    );
    assert_eq!(
        policy("-- rupa:\nSELECT 1;").expect("empty frontmatter"),
        Policy::TransactionalRequired
    );
    assert_eq!(
        Frontmatter::parse("rupa: {}")
            .expect("empty mapping")
            .policy,
        Policy::TransactionalRequired
    );
}

#[test]
fn unknown_repeated_or_malformed_frontmatter_fails() {
    let sources = [
        "-- rupa:\n--   autocommit: true\nSELECT 1;",
        "-- rupa:\n--   policy: sometimes\nSELECT 1;",
        "-- rupa: nontransactional\nSELECT 1;",
        "-- rupa:\n--   policy: [unclosed\nSELECT 1;",
        "-- rupa:\n--   policy: transactional\n--   policy: nontransactional\nSELECT 1;",
        "-- rupa: {}\n-- rupa: { policy: nontransactional }\nSELECT 1;",
    ];

    for source in sources {
        let result = policy(source);

        assert!(
            matches!(&result, Err(Error::Frontmatter { path, .. }) if path == Path::new("1_x.sql")),
            "{source:?}: {result:?}"
        );
    }
}
