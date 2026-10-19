mod postgres;
mod sqlite;

use super::*;

#[test]
fn rejects_dialects_without_a_script_grammar() {
    let dialects = [DialectId::MySql, DialectId::Memory];

    for dialect in dialects {
        assert_eq!(
            validate_script("SELECT 1", dialect),
            Err(ScriptError::Dialect(dialect)),
            "{dialect:?}"
        );
    }
}
