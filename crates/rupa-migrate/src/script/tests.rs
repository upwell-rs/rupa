use super::*;

#[test]
fn validates_quotes_comments_and_semicolons_without_splitting() {
    let scripts = [
        "SELECT 'a;''b', \"a;\"\"b\", `x``y`, [a;b]; -- hi\n",
        "/* ' ignored */ SELECT 1;",
        "SELECT 'é'; -- tail",
    ];

    for script in scripts {
        assert!(validate(script).is_ok(), "{script}");
    }
}

#[test]
fn rejects_unterminated_quotes_and_comments_before_dispatch() {
    let scripts = [
        "SELECT 'oops",
        "SELECT \"oops",
        "SELECT [oops",
        "SELECT `oops",
        "SELECT 1; /* oops",
    ];

    for script in scripts {
        assert!(validate(script).is_err(), "{script}");
    }
}
