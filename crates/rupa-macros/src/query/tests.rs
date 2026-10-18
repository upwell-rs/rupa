use super::*;

fn text(s: &str) -> SqlPart {
    SqlPart::Text(s.into())
}
fn param(s: &str) -> SqlPart {
    SqlPart::Param(s.into())
}

#[test]
fn splits_named_parameters() {
    assert_eq!(
        split_sql("SELECT * FROM t WHERE a = $a AND b > $b_2").unwrap(),
        [
            text("SELECT * FROM t WHERE a = "),
            param("a"),
            text(" AND b > "),
            param("b_2")
        ]
    );
    assert_eq!(split_sql("$x").unwrap(), [param("x")]);
}

#[test]
fn ignores_dollars_in_quotes_and_comments() {
    let sql = "SELECT '$no', \"$col\" -- $c\n /* $d */ FROM t WHERE x = $yes";
    let parts = split_sql(sql).unwrap();
    assert_eq!(
        parts
            .iter()
            .filter(|p| matches!(p, SqlPart::Param(_)))
            .count(),
        1
    );
    assert_eq!(parts.last(), Some(&param("yes")));
}

#[test]
fn rejects_dialect_dependent_backslash_quotes() {
    let statements = [
        r"SELECT 'O\'Brien $literal', $yes",
        r"SELECT E'O\'Brien $literal', $yes",
        r"SELECT 'a\' || $literal || \'b', $yes",
        r#"SELECT "a\"$literal", $yes"#,
        r"SELECT '\', $yes",
        r"SELECT 'a\\' AS value, $yes",
    ];

    for sql in statements {
        let error = split_sql(sql).expect_err("ambiguous quoting must not extract parameters");

        assert!(error.contains("backslash before a quote"), "{error}");
        assert!(error.contains("bind"), "{error}");
    }
}

#[test]
fn preserves_portable_quotes_and_literal_backslashes() {
    let literals = [
        r"'O''Brien $literal'",
        r#""a""$literal""#,
        r"'C:\data $literal'",
        "'東京 $literal'",
    ];

    for literal in literals {
        let sql = format!("SELECT {literal}, $yes");
        let parts = split_sql(&sql).expect("portable quoted SQL should split");

        assert_eq!(parts, [text(&format!("SELECT {literal}, ")), param("yes")]);
    }
}

#[test]
fn rejects_unsupported_placeholders() {
    assert!(
        split_sql("WHERE a = $1")
            .unwrap_err()
            .contains("positional")
    );
    assert!(
        split_sql("SELECT $tag$x$tag$")
            .unwrap_err()
            .contains("dollar-quoted")
    );
    assert!(split_sql("WHERE a = $").is_err());
    assert!(split_sql("WHERE a = 'open").is_err());
}
