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
