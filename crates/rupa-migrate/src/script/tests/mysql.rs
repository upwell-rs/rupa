use super::*;

fn validate(sql: &str) -> Result<(), ScriptError> {
    validate_script(sql, DialectId::MySql)
}

fn split(sql: &str) -> Vec<&str> {
    split_script(sql, DialectId::MySql)
        .expect("valid script")
        .into_iter()
        .map(|range| &sql[range])
        .collect()
}

fn forbidden(rule: ScriptRule, at: usize) -> Result<(), ScriptError> {
    Err(ScriptError::Forbidden { rule, at })
}

#[test]
fn keeps_quotes_identifiers_and_comments_as_single_units() {
    let scripts = [
        "SELECT 'a;''b', \"a;\"\"b\", `a;``b` FROM t;",
        "SELECT 'C:\\'; SELECT 1;",
        "SELECT 1; # COMMIT;\nSELECT 2; -- ROLLBACK;\n",
        "/* COMMIT; */ SELECT 1; /*+ hint */",
        "SELECT 5--1;",
        "CREATE FUNCTION f() RETURNS INT DETERMINISTIC RETURN 1;",
        "CREATE TRIGGER t BEFORE INSERT ON a FOR EACH ROW SET NEW.x = 1;",
        "UPDATE t SET a = 1; ALTER TABLE t ALGORITHM=INPLACE, LOCK=NONE, ADD INDEX i (a);",
        "SELECT IS_USED_LOCK('x'), @v, @@session.sql_mode, _utf8mb4'x', X'0F';",
        "CREATE TABLE notes (id BIGINT, `begin` INT, price$ INT);",
    ];

    for script in scripts {
        assert_eq!(validate(script), Ok(()), "{script}");
    }
}

#[test]
fn rejects_unterminated_quotes_and_comments() {
    let scripts = [
        ("SELECT 'oops", 7),
        ("SELECT \"oops", 7),
        ("SELECT `oops", 7),
        ("SELECT 1; /* a", 10),
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
            "CREATE TABLE a (id INT); COMMIT;",
            ScriptRule::TransactionControl,
            25,
        ),
        ("BEGIN", ScriptRule::TransactionControl, 0),
        ("START TRANSACTION", ScriptRule::TransactionControl, 0),
        ("ROLLBACK", ScriptRule::TransactionControl, 0),
        ("SAVEPOINT s", ScriptRule::TransactionControl, 0),
        ("RELEASE SAVEPOINT s", ScriptRule::TransactionControl, 0),
        ("XA START 'x'", ScriptRule::TransactionControl, 0),
        ("LOCK TABLES t WRITE", ScriptRule::TransactionControl, 0),
        ("UNLOCK TABLES", ScriptRule::TransactionControl, 0),
        ("SET sql_mode = ''", ScriptRule::SessionState, 0),
        ("SET @@autocommit = 0", ScriptRule::SessionState, 0),
        ("USE other", ScriptRule::SessionState, 0),
        ("DELIMITER //", ScriptRule::ClientCommand, 0),
        ("SOURCE other.sql", ScriptRule::ClientCommand, 0),
        (
            "LOAD DATA LOCAL INFILE 'x' INTO TABLE t",
            ScriptRule::ClientCommand,
            0,
        ),
        ("SELECT 1\\G", ScriptRule::ClientCommand, 8),
        ("SELECT GET_LOCK('x', 1)", ScriptRule::AdvisoryLock, 7),
        ("DO release_all_locks()", ScriptRule::AdvisoryLock, 3),
        ("DELETE FROM _rupa_migrations", ScriptRule::ReservedName, 12),
        (
            "RENAME TABLE t TO `_RUPA_hidden`",
            ScriptRule::ReservedName,
            18,
        ),
        (
            "SELECT 1 /*! , SLEEP(1) */",
            ScriptRule::ExecutableComment,
            9,
        ),
        ("/*M!100000 COMMIT */", ScriptRule::ExecutableComment, 0),
        (
            "CREATE PROCEDURE p() BEGIN SELECT 1; END",
            ScriptRule::CompoundStatement,
            21,
        ),
    ];

    for (script, rule, at) in scripts {
        assert_eq!(validate(script), forbidden(rule, at), "{script}");
    }
}

#[test]
fn splits_statements_at_unquoted_semicolons_only() {
    let script = "CREATE TABLE a (s TEXT, `x;y` INT);\n# ; comment\nINSERT INTO a VALUES (';', 1); -- tail ;\n";

    assert_eq!(
        split(script),
        [
            "CREATE TABLE a (s TEXT, `x;y` INT)",
            "\n# ; comment\nINSERT INTO a VALUES (';', 1)",
        ]
    );
}
