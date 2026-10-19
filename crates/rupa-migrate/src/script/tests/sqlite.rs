use super::*;

#[test]
fn validates_quotes_comments_and_semicolons_without_splitting() {
    let scripts = [
        "SELECT 'a;''b', \"a;\"\"b\", `x``y`, [a;b]; -- hi\n",
        "/* ' ignored */ SELECT 1;",
        "SELECT 'é'; -- tail",
        "SELECT 'a\\'; SELECT 1;",
        "SELECT 1; /**/",
        "CREATE TRIGGER t AFTER INSERT ON a BEGIN DELETE FROM b; END; COMMIT;",
    ];

    for script in scripts {
        assert_eq!(
            validate_script(script, DialectId::Sqlite),
            Ok(()),
            "{script}"
        );
    }
}

#[test]
fn rejects_unterminated_quotes_and_comments_before_dispatch() {
    let scripts = [
        ("SELECT 'oops", 7),
        ("SELECT \"oops", 7),
        ("SELECT [oops", 7),
        ("SELECT `oops", 7),
        ("SELECT 1; /* oops", 10),
        ("SELECT 1; /*/", 10),
    ];

    for (script, offset) in scripts {
        assert_eq!(
            validate_script(script, DialectId::Sqlite),
            Err(ScriptError::Unterminated(offset)),
            "{script}"
        );
    }
}
