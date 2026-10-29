use super::action::*;
use super::authorize;

#[test]
fn user_objects_in_main_are_allowed() {
    assert!(authorize(CREATE_TABLE, Some("notes"), None, Some("main")));
    assert!(authorize(READ, Some("notes"), Some("title"), Some("main")));
    assert!(authorize(
        CREATE_INDEX,
        Some("notes_title"),
        Some("notes"),
        Some("main")
    ));
    assert!(authorize(FUNCTION, None, Some("lower"), None));
    assert!(authorize(SELECT, None, None, None));
}

#[test]
fn reserved_objects_are_refused_in_any_case() {
    assert!(!authorize(
        INSERT,
        Some("_rupa_migrations"),
        None,
        Some("main")
    ));
    assert!(!authorize(
        READ,
        Some("_RUPA_Migrations"),
        Some("owner"),
        Some("main")
    ));
    assert!(!authorize(
        CREATE_INDEX,
        Some("notes_title"),
        Some("_rupa_migrations"),
        Some("main")
    ));
    assert!(!authorize(
        CREATE_TRIGGER,
        Some("_rupa_trigger"),
        Some("notes"),
        Some("main")
    ));
    assert!(!authorize(
        ALTER_TABLE,
        Some("main"),
        Some("_rupa_migrations"),
        None
    ));
}

#[test]
fn alter_table_names_its_database_first_and_a_dropped_column_last() {
    assert!(authorize(ALTER_TABLE, Some("main"), Some("notes"), None));
    assert!(
        authorize(ALTER_TABLE, Some("main"), Some("notes"), Some("slug")),
        "DROP COLUMN passes the column where other actions pass the database"
    );
    assert!(!authorize(ALTER_TABLE, Some("temp"), Some("notes"), None));
    assert!(!authorize(
        ALTER_TABLE,
        Some("aux"),
        Some("notes"),
        Some("main")
    ));
}

#[test]
fn other_databases_control_and_extensions_are_refused() {
    const TRANSACTION: i32 = 22;
    const ATTACH: i32 = 24;
    const PRAGMA: i32 = 19;

    assert!(!authorize(CREATE_TABLE, Some("notes"), None, Some("aux")));
    assert!(!authorize(CREATE_TABLE, Some("notes"), None, Some("temp")));
    assert!(authorize(
        READ,
        Some("sqlite_temp_master"),
        Some("sql"),
        Some("temp")
    ));
    assert!(!authorize(TRANSACTION, Some("COMMIT"), None, None));
    assert!(!authorize(ATTACH, Some("other.db"), None, None));
    assert!(!authorize(PRAGMA, Some("writable_schema"), Some("1"), None));
    assert!(!authorize(FUNCTION, None, Some("LOAD_EXTENSION"), None));
}
