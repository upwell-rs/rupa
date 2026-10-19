use rupa::migrate::Session;

use super::fixture::{RawCommit, body};
use super::{MigrationHarness, catalog};

const SHARED: [&str; 6] = [
    "CREATE TABLE marker(id INTEGER); COMMIT;",
    "CREATE TABLE marker(id INTEGER); ROLLBACK;",
    "CREATE TABLE marker(id INTEGER); SAVEPOINT escape;",
    "CREATE TABLE marker(id INTEGER); DELETE FROM _rupa_migration_history;",
    "CREATE TABLE marker(id INTEGER); DELETE FROM _RUPA_MIGRATION_ATTEMPTS;",
    "CREATE TABLE marker(id INTEGER); SELECT * FROM _rupa_migration_history;",
];

pub(super) fn case_scripts_cannot_escape_the_runner<H: MigrationHarness>(h: &mut H) {
    let scripts = SHARED.iter().chain(h.forbidden_scripts());

    for sql in scripts {
        let entries = [body("1", sql)];

        let mut session = h.connect();

        assert!(session.apply(&catalog(&entries)).is_err(), "{sql}");
        assert!(
            session.history().expect("no committed body").is_empty(),
            "{sql}"
        );
        assert!(!h.table_exists("marker"), "{sql}");
    }
}

pub(super) fn case_raw_queries_cannot_escape_the_runner<H: MigrationHarness>(h: &mut H) {
    let entries = [RawCommit];

    let mut session = h.connect();

    assert!(session.apply(&catalog(&entries)).is_err());
    assert!(session.history().expect("empty history").is_empty());
    assert!(!h.table_exists("marker"));
}

pub(super) fn case_renaming_into_reserved_names_cannot_commit<H: MigrationHarness>(h: &mut H) {
    let entries = [body(
        "1",
        "CREATE TABLE notes(id INTEGER); ALTER TABLE notes RENAME TO _rupa_hidden;",
    )];

    assert!(h.connect().apply(&catalog(&entries)).is_err());
    assert!(!h.table_exists("notes"));
    assert!(!h.table_exists("_rupa_hidden"));
}
