use super::*;

fn validate(sql: &str) -> Result<(), ScriptError> {
    validate_script(sql, DialectId::Postgres)
}

fn forbidden(rule: ScriptRule, at: usize) -> Result<(), ScriptError> {
    Err(ScriptError::Forbidden { rule, at })
}

#[test]
fn keeps_quotes_comments_and_dollar_bodies_as_single_units() {
    let scripts = [
        "SELECT 'a;''b', \"a;\"\"b\"; -- COMMIT\n",
        "SELECT E'it\\'s; \\\\', e'x''y', U&'d\\0061t;a';",
        "/* outer /* COMMIT; */ still comment */ SELECT 1;",
        "CREATE FUNCTION f() RETURNS void AS $$ BEGIN COMMIT; END $$ LANGUAGE plpgsql;",
        "DO $body$ BEGIN PERFORM 1; $$ nested $$; END $body$;",
        "SELECT $1, price$, a$b FROM t WHERE x = $2;",
        "SET LOCAL lock_timeout = '1s'; SET CONSTRAINTS ALL DEFERRED;",
        "SET TRANSACTION ISOLATION LEVEL SERIALIZABLE; SELECT 'set_config';",
        "UPDATE t SET a = 1; ALTER TABLE t ALTER COLUMN a SET DEFAULT 0;",
        "PREPARE q AS SELECT 1; COPY t FROM '/srv/data.csv';",
        "CREATE TABLE notes (id bigint, \"end\" text, rollback_count int);",
    ];

    for script in scripts {
        assert_eq!(validate(script), Ok(()), "{script}");
    }
}

#[test]
fn rejects_unterminated_quotes_comments_and_bodies() {
    let scripts = [
        ("SELECT 'oops", 7),
        ("SELECT E'it\\'s", 7),
        ("SELECT \"oops", 7),
        ("SELECT 1; /* a /* b */", 10),
        ("SELECT $$oops", 7),
        ("SELECT $a$ body $b$", 7),
    ];

    for (script, offset) in scripts {
        assert_eq!(
            validate(script),
            Err(ScriptError::Unterminated(offset)),
            "{script}"
        );
    }
}

#[test]
fn rejects_runner_owned_statement_classes() {
    let scripts = [
        (
            "CREATE TABLE a (id int); COMMIT;",
            ScriptRule::TransactionControl,
            25,
        ),
        ("begin; SELECT 1", ScriptRule::TransactionControl, 0),
        ("START TRANSACTION", ScriptRule::TransactionControl, 0),
        ("SELECT 1;\nROLLBACK", ScriptRule::TransactionControl, 10),
        ("END", ScriptRule::TransactionControl, 0),
        ("SAVEPOINT s", ScriptRule::TransactionControl, 0),
        ("RELEASE s", ScriptRule::TransactionControl, 0),
        ("PREPARE TRANSACTION 'x'", ScriptRule::TransactionControl, 0),
        ("SET search_path = other", ScriptRule::SessionState, 0),
        (
            "SET SESSION AUTHORIZATION other",
            ScriptRule::SessionState,
            0,
        ),
        ("SET \"search_path\" = other", ScriptRule::SessionState, 0),
        ("RESET ALL", ScriptRule::SessionState, 0),
        (
            "SELECT set_config('search_path', 'other', false)",
            ScriptRule::SessionState,
            7,
        ),
        ("UNLISTEN *", ScriptRule::SessionState, 0),
        ("COPY t TO STDOUT", ScriptRule::ClientCommand, 0),
        ("DISCARD ALL", ScriptRule::SessionState, 0),
        ("LISTEN events", ScriptRule::SessionState, 0),
        ("COPY t FROM STDIN", ScriptRule::ClientCommand, 0),
        ("\\i other.sql", ScriptRule::ClientCommand, 0),
        (
            "SELECT pg_advisory_unlock_all()",
            ScriptRule::AdvisoryLock,
            7,
        ),
        (
            "SELECT PG_TRY_ADVISORY_LOCK(1)",
            ScriptRule::AdvisoryLock,
            7,
        ),
        (
            "DELETE FROM _rupa_migration_history",
            ScriptRule::ReservedName,
            12,
        ),
        (
            "ALTER TABLE t RENAME TO \"_RUPA_hidden\"",
            ScriptRule::ReservedName,
            24,
        ),
        (
            "CREATE FUNCTION f() RETURNS int LANGUAGE sql BEGIN ATOMIC SELECT 1; END",
            ScriptRule::AtomicBody,
            51,
        ),
    ];

    for (script, rule, at) in scripts {
        assert_eq!(validate(script), forbidden(rule, at), "{script}");
    }
}
