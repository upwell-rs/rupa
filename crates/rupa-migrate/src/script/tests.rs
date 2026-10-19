use super::*;

#[test]
fn validates_quotes_comments_and_semicolons_without_splitting() {
    let scripts = [
        "SELECT 'a;''b', \"a;\"\"b\", `x``y`, [a;b]; -- hi\n",
        "/* ' ignored */ SELECT 1;",
        "SELECT 'é'; -- tail",
        "SELECT 'a\\'; SELECT 1;",
        "SELECT 1; /**/",
    ];

    for script in scripts {
        assert!(validate(script, DialectId::Sqlite).is_ok(), "{script}");
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
        assert!(
            matches!(validate(script, DialectId::Sqlite), Err(ValidationError::Sql(at)) if at == offset),
            "{script}"
        );
    }
}

#[test]
fn rejects_dialects_without_a_script_grammar() {
    let dialects = [DialectId::Postgres, DialectId::MySql, DialectId::Memory];

    for dialect in dialects {
        assert!(
            matches!(
                validate("SELECT $$it's$$", dialect),
                Err(ValidationError::ScriptDialect(rejected)) if rejected == dialect
            ),
            "{dialect:?}"
        );
    }
}
