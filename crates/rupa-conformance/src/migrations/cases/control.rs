use super::{Harness, Session, catalog};
use crate::migrations::fixture::{RawCommit, body};

const SHARED: [&str; 6] = [
    "CREATE TABLE marker(id INTEGER); COMMIT;",
    "CREATE TABLE marker(id INTEGER); ROLLBACK;",
    "CREATE TABLE marker(id INTEGER); SAVEPOINT escape;",
    "CREATE TABLE marker(id INTEGER); DELETE FROM _rupa_migration_history;",
    "CREATE TABLE marker(id INTEGER); DELETE FROM _RUPA_MIGRATION_ATTEMPTS;",
    "CREATE TABLE marker(id INTEGER); SELECT * FROM _rupa_migration_history;",
];

pub(super) fn case_scripts_cannot_escape_the_runner<H: Harness>(h: &mut H) {
    let scripts = SHARED.iter().chain(h.forbidden_scripts());

    for sql in scripts {
        let entries = [body("1", sql)];

        let mut session = h.connect();

        assert!(
            wait!(h, session.apply(&catalog(&entries))).is_err(),
            "{sql}"
        );
        assert!(
            wait!(h, session.history())
                .expect("no committed body")
                .is_empty(),
            "{sql}"
        );
        assert!(!h.table_exists("marker"), "{sql}");
    }
}

pub(super) fn case_raw_queries_cannot_escape_the_runner<H: Harness>(h: &mut H) {
    let entries = [RawCommit];

    let mut session = h.connect();

    assert!(wait!(h, session.apply(&catalog(&entries))).is_err());
    assert!(
        wait!(h, session.history())
            .expect("empty history")
            .is_empty()
    );
    assert!(!h.table_exists("marker"));
}

pub(super) fn case_renaming_into_reserved_names_cannot_commit<H: Harness>(h: &mut H) {
    let entries = [body(
        "1",
        "CREATE TABLE notes(id INTEGER); ALTER TABLE notes RENAME TO _rupa_hidden;",
    )];

    assert!(wait!(h, h.connect().apply(&catalog(&entries))).is_err());
    assert!(!h.table_exists("notes"));
    assert!(!h.table_exists("_rupa_hidden"));
}
