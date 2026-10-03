use rupa_core::dialect::{DialectId, DynDialect, Postgres};
use rupa_core::ir::{ColumnRef, ExprNode};
use rupa_core::{DslError, Expr, Value};
use rupa_dsl_std::*;
use serde_json::json;

fn col<T>(name: &'static str) -> Expr<T> {
    Expr::from_node(ExprNode::Column(ColumnRef::new(name)))
}

/// `SELECT 1 FROM "t" WHERE <e>`, rendered for Postgres, with its params.
fn pg<T>(e: Expr<T>) -> String {
    use rupa_core::ir::{FromClause, Select, Statement, TableRef};
    let stmt = Statement::Select(Select {
        from: FromClause::Table(TableRef::new("t")),
        projection: vec![],
        filter: Some(e.into_node()),
        order_by: vec![],
        limit: None,
        offset: None,
    });
    let r = rupa_sql::render(&stmt, &Postgres).unwrap();
    let params: Vec<String> = r.params.iter().map(|p| format!("{p:?}")).collect();
    format!("{}  -- {}", r.sql, params.join(", "))
}

#[test]
fn postgres_lowerings() {
    let out = [
        pg(ilike(col::<String>("email"), "%@x")),
        pg(json_contains(
            col::<serde_json::Value>("prefs"),
            json!({"beta": true}),
        )),
        pg(json_has_key(col::<serde_json::Value>("prefs"), "theme")),
        pg(lower(col::<String>("email"))),
        pg(upper(lower(col::<String>("email")))),
    ];
    insta::assert_snapshot!(out.join(
        "
"
    ));
}

fn lowered(e: &Expr<bool>, dialect: DialectId) -> Result<ExprNode, DslError> {
    match e.node() {
        ExprNode::Dsl(call) => call.lower(&DynDialect::new(dialect)),
        _ => unreachable!(),
    }
}

#[test]
fn ilike_branches_per_dialect_at_runtime() {
    let e = ilike(col::<String>("email"), "%@x");
    assert!(matches!(
        lowered(&e, DialectId::Postgres),
        Ok(ExprNode::RawOp("ILIKE", ..))
    ));
    for d in [DialectId::MySql, DialectId::Sqlite] {
        assert!(
            matches!(lowered(&e, d), Ok(ExprNode::RawOp("LIKE", ..))),
            "{d:?}"
        );
    }
    assert_eq!(
        lowered(&e, DialectId::Memory),
        Err(DslError::unsupported("ilike", DialectId::Memory))
    );
}

#[test]
fn static_gate_is_rechecked_when_lowering() {
    let e = json_contains(col::<serde_json::Value>("prefs"), json!({}));
    // Sqlite does not declare JsonContainment: refused before the body runs.
    assert_eq!(
        lowered(&e, DialectId::Sqlite),
        Err(DslError::unsupported("json_contains", DialectId::Sqlite))
    );
}

#[test]
fn evals() {
    let eval = |e: &Expr<bool>, args: &[Value]| match e.node() {
        ExprNode::Dsl(call) => (call.def.eval.expect("has eval"))(args),
        _ => unreachable!(),
    };
    let e = ilike(col::<String>("email"), "x");
    assert_eq!(
        eval(
            &e,
            &[Value::Text("Ada@X".into()), Value::Text("%@x".into())]
        ),
        Ok(Value::Bool(true))
    );
    assert_eq!(
        eval(
            &e,
            &[
                Value::Null(rupa_core::SqlType::Text),
                Value::Text("%".into())
            ]
        ),
        Ok(Value::Null(rupa_core::SqlType::Bool))
    );
    let c = json_contains(col::<serde_json::Value>("p"), json!({}));
    assert_eq!(
        eval(
            &c,
            &[
                Value::Json(json!({"a": [1, 2]})),
                Value::Json(json!({"a": [2]}))
            ]
        ),
        Ok(Value::Bool(true))
    );
    let k = json_has_key(col::<serde_json::Value>("p"), "a");
    assert_eq!(
        eval(
            &k,
            &[Value::Json(json!({"a": null})), Value::Text("a".into())]
        ),
        Ok(Value::Bool(true))
    );
    assert_eq!(
        eval(&k, &[Value::Json(json!([1])), Value::Text("a".into())]),
        Ok(Value::Bool(false))
    );
}
