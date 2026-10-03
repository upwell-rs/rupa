//! Snapshot tests: rendered SQL + params for every IR node, Postgres dialect.

// The entity fixture lives with rupa-core's tests; shared to avoid drift.
#[path = "../../rupa-core/tests/fixtures/mod.rs"]
mod fixtures;

use std::fmt::Write as _;

use fixtures::{Prefs, User};
use rupa_core::dialect::{DialectId, DynDialect, Postgres};
use rupa_core::ir::{
    ColumnRef, Delete, DslFnDef, ExprNode, FromClause, PathSeg, RawPart, RawSql, Select, Statement,
    TableRef, UnOp,
};
use rupa_core::prelude::*;
use rupa_core::{Dialect, DslError, Value};
use rupa_sql::{RenderError, render, render_query};

fn pg<R>(q: &Query<R>) -> String {
    show(render_query(q, &Postgres).expect("render"))
}

fn pg_stmt(s: &Statement) -> String {
    show(render(s, &Postgres).expect("render"))
}

fn show(r: rupa_sql::Rendered) -> String {
    let mut out = r.sql;
    for (i, p) in r.params.iter().enumerate() {
        let _ = write!(out, "\n${} = {p:?}", i + 1);
    }
    out
}

fn where_(cond: Expr<bool>) -> Query<Vec<User>> {
    select::<User>().filter(cond).all()
}

fn bool_expr(node: ExprNode) -> Expr<bool> {
    Expr::from_node(node)
}

// --- statements -------------------------------------------------------------

#[test]
fn select_all() {
    insta::assert_snapshot!(pg(&select::<User>().all()));
}

#[test]
fn select_order_limit_offset() {
    let q = select::<User>()
        .filter(User::ACTIVE.eq(true))
        .order_by(User::CREATED_AT.desc())
        .order_by(User::ID.asc())
        .limit(50)
        .offset(10)
        .all();
    insta::assert_snapshot!(pg(&q));
}

#[test]
fn select_without_schema_and_empty_projection() {
    let s = Statement::Select(Select {
        from: FromClause::Table(TableRef::new("plain")),
        projection: vec![],
        filter: None,
        order_by: vec![],
        limit: None,
        offset: None,
    });
    insta::assert_snapshot!(pg_stmt(&s));
}

#[test]
fn exists() {
    insta::assert_snapshot!(pg(&select::<User>()
        .filter(User::EMAIL.eq("a@b.c"))
        .exists()));
}

#[test]
fn insert_one() {
    insta::assert_snapshot!(pg(&insert::<User>().value(&User::sample()).affected()));
}

#[test]
fn insert_many_with_nulls() {
    let mut second = User::sample();
    second.id = 8;
    second.nickname = Some("bee".into());
    second.old_prefs = Some(Prefs {
        theme: "light".into(),
        beta: false,
    });
    insta::assert_snapshot!(pg(&insert::<User>()
        .values([&User::sample(), &second])
        .affected()));
}

#[test]
fn insert_returning() {
    insta::assert_snapshot!(pg(&insert::<User>().value(&User::sample()).returning_one()));
}

#[test]
fn get_by_id() {
    insta::assert_snapshot!(pg(&get::<User>(&7)));
}

#[test]
fn update_one_and_delete_by_id() {
    let sql = pg(&update::<User>().one(&User::sample()).affected())
        + "
---
" + &pg(&delete::<User>().by_id(&7).affected());
    insta::assert_snapshot!(sql);
}

#[test]
fn update_set_and_filter() {
    let q = update::<User>()
        .set(
            User::PREFS,
            &Prefs {
                theme: "light".into(),
                beta: false,
            },
        )
        .set(User::OLD_PREFS, &None)
        .set_expr(User::NICKNAME, User::EMAIL)
        .filter(User::ID.eq(7))
        .affected();
    insta::assert_snapshot!(pg(&q));
}

#[test]
fn delete_filtered() {
    insta::assert_snapshot!(pg(&delete::<User>()
        .filter(User::ID.in_([1i64, 2, 3]))
        .affected()));
}

#[test]
fn delete_unfiltered_without_schema() {
    let s = Statement::Delete(Delete {
        table: TableRef::new("plain"),
        filter: None,
    });
    insta::assert_snapshot!(pg_stmt(&s));
}

#[test]
fn raw_sql() {
    let q = raw()
        .sql("SELECT * FROM app.users WHERE email = ")
        .bind("a@b.c".to_string())
        .sql(" AND id > ")
        .bind(5i64)
        .rows::<Vec<User>>();
    insta::assert_snapshot!(pg(&q));
}

// --- expressions ------------------------------------------------------------

#[test]
fn comparisons() {
    let cond = User::ID
        .eq(1)
        .and(User::ID.ne(2))
        .and(User::ID.lt(3))
        .and(User::ID.le(4))
        .and(User::ID.gt(5))
        .and(User::ID.ge(6));
    insta::assert_snapshot!(pg(&where_(cond)));
}

#[test]
fn column_to_column() {
    insta::assert_snapshot!(pg(&where_(User::NICKNAME.eq(User::EMAIL))));
}

#[test]
fn like() {
    insta::assert_snapshot!(pg(&where_(User::EMAIL.like("%@example.com"))));
}

#[test]
fn in_and_not_in() {
    let cond = User::EMAIL.in_(["a", "b"]).and(User::ID.not_in([1i64, 2]));
    insta::assert_snapshot!(pg(&where_(cond)));
}

#[test]
fn in_empty_lists() {
    let cond = User::ID
        .in_(Vec::<i64>::new())
        .or(User::ID.not_in(Vec::<i64>::new()));
    insta::assert_snapshot!(pg(&where_(cond)));
}

#[test]
fn null_tests() {
    let cond = User::NICKNAME
        .is_null()
        .and(User::OLD_PREFS.is_not_null())
        .and(User::PREFS.is_null());
    insta::assert_snapshot!(pg(&where_(cond)));
}

#[test]
fn precedence_or_inside_and_is_parenthesized() {
    let cond = User::EMAIL
        .eq("a")
        .or(User::EMAIL.eq("b"))
        .and(User::ACTIVE.eq(true));
    insta::assert_snapshot!(pg(&where_(cond)));
}

#[test]
fn precedence_and_inside_or_is_not() {
    let cond = User::EMAIL
        .eq("a")
        .and(User::ACTIVE.eq(true))
        .or(User::ID.eq(1));
    insta::assert_snapshot!(pg(&where_(cond)));
}

#[test]
fn not() {
    let cond = !User::EMAIL.eq("a").or(User::ACTIVE.eq(false));
    insta::assert_snapshot!(
        pg(&where_(!User::ACTIVE.eq(true)).clone()) + "\n---\n" + &pg(&where_(cond))
    );
}

#[test]
fn not_of_bool_column_and_comparison_of_comparison() {
    // A comparison as an operand of a comparison is parenthesized.
    let cmp = Expr::<bool>::from_node(User::ID.eq(1).into_node());
    insta::assert_snapshot!(pg(&where_(!bind(true))) + "\n---\n" + &pg(&where_(cmp.eq(true))));
}

#[test]
fn json_path_text() {
    insta::assert_snapshot!(pg(&where_(User::PREFS.path("theme").text().eq("dark"))));
}

#[test]
fn json_path_nested_and_index() {
    let cond = User::PREFS.path("a").index(0).key("b").text().is_not_null();
    insta::assert_snapshot!(pg(&where_(cond)));
}

#[test]
fn json_path_as_json() {
    let node = ExprNode::Unary(
        UnOp::IsNull,
        Box::new(User::PREFS.path("a").key("b").json().into_node()),
    );
    insta::assert_snapshot!(pg(&where_(bool_expr(node))));
}

#[test]
fn json_path_cast() {
    insta::assert_snapshot!(pg(&where_(
        User::PREFS.path("beta").cast::<bool>().eq(true)
    )));
}

#[test]
fn json_key_escaping() {
    let cond = User::PREFS
        .path("it's")
        .text()
        .eq("x")
        .and(User::PREFS.path("back\\slash").text().eq("y"));
    insta::assert_snapshot!(pg(&where_(cond)));
}

#[test]
fn identifier_quoting() {
    let node = ExprNode::Unary(
        UnOp::IsNull,
        Box::new(ExprNode::Column(ColumnRef::new("we\"ird"))),
    );
    let s = Statement::Delete(Delete {
        table: TableRef::with_schema("my schema", "t\"1"),
        filter: Some(node),
    });
    insta::assert_snapshot!(pg_stmt(&s));
}

#[test]
fn raw_op_and_call() {
    let json_param = Expr::<serde_json::Value>::from_node(ExprNode::Param(Value::Json(
        serde_json::json!({"x": 1}),
    )));
    let raw = Expr::<bool>::raw_op("@>", User::PREFS.path("a").json(), json_param);
    let nickname = Expr::<Option<String>>::from_node(ExprNode::Column(ColumnRef::new("nickname")));
    let call = Expr::<Option<String>>::call("lower", [nickname]);
    insta::assert_snapshot!(pg(&where_(raw.and(call.eq("x")))));
}

#[test]
fn json_path_as_raw_op_operand_is_parenthesized() {
    let json_param = Expr::<serde_json::Value>::from_node(ExprNode::Param(Value::Json(
        serde_json::json!({"x": 1}),
    )));
    let raw = Expr::<bool>::raw_op("<@", json_param, User::PREFS.path("a").json());
    insta::assert_snapshot!(pg(&where_(raw)));
}

#[test]
fn typed_nulls() {
    let q = raw()
        .bind(None::<i64>)
        .sql(", ")
        .bind_value(Value::Null(rupa_core::SqlType::Json))
        .execute::<u64>();
    insta::assert_snapshot!(pg(&q));
}

// --- DSL calls: lowered at render time --------------------------------------

fn lower_ilike(d: &dyn Dialect, mut args: Vec<ExprNode>) -> Result<ExprNode, DslError> {
    let (r, l) = (args.pop().unwrap(), args.pop().unwrap());
    match d.id() {
        DialectId::Postgres => Ok(ExprNode::RawOp("ILIKE", Box::new(l), Box::new(r))),
        other => Err(DslError::unsupported("ilike", other)),
    }
}
static ILIKE: DslFnDef = DslFnDef {
    name: "ilike",
    lower: lower_ilike,
    eval: None,
};

fn lower_ci_like(_: &dyn Dialect, mut args: Vec<ExprNode>) -> Result<ExprNode, DslError> {
    let (r, l) = (args.pop().unwrap(), args.pop().unwrap());
    Ok(ExprNode::Binary(
        rupa_core::ir::BinOp::Like,
        Box::new(ExprNode::Call("LOWER", vec![l])),
        Box::new(ExprNode::Call("LOWER", vec![r])),
    ))
}
static CI_LIKE: DslFnDef = DslFnDef {
    name: "ci_like",
    lower: lower_ci_like,
    eval: None,
};

fn lower_never(d: &dyn Dialect, _: Vec<ExprNode>) -> Result<ExprNode, DslError> {
    Err(DslError::unsupported("never", d.id()))
}
static NEVER: DslFnDef = DslFnDef {
    name: "never",
    lower: lower_never,
    eval: None,
};

fn lower_cycle(_: &dyn Dialect, args: Vec<ExprNode>) -> Result<ExprNode, DslError> {
    Ok(Expr::<bool>::dsl(&CYCLE, args).into_node())
}
static CYCLE: DslFnDef = DslFnDef {
    name: "cycle",
    lower: lower_cycle,
    eval: None,
};

fn dsl(def: &'static DslFnDef) -> Expr<bool> {
    Expr::dsl(
        def,
        vec![
            ExprNode::Column(ColumnRef::new("email")),
            ExprNode::Param(Value::Text("%A%".into())),
        ],
    )
}

#[test]
fn dsl_lowered_to_raw_op() {
    insta::assert_snapshot!(pg(&where_(dsl(&ILIKE).and(User::ACTIVE.eq(true)))));
}

#[test]
fn dsl_lowered_under_not_is_parenthesized() {
    insta::assert_snapshot!(pg(&where_(!dsl(&ILIKE))));
}

#[test]
fn dsl_lowered_to_calls() {
    insta::assert_snapshot!(pg(&where_(dsl(&CI_LIKE))));
}

#[test]
fn dsl_errors_surface_at_render() {
    assert_eq!(
        render_query(&where_(dsl(&NEVER)), &Postgres),
        Err(RenderError::Dsl(DslError::unsupported(
            "never",
            DialectId::Postgres
        )))
    );
    assert_eq!(
        render_query(&where_(dsl(&CYCLE)), &Postgres),
        Err(RenderError::TooDeep)
    );
}

// --- errors and dialect selection -------------------------------------------

#[test]
fn render_errors() {
    let bad_op = bool_expr(ExprNode::RawOp(
        "= 1; --",
        Box::new(ExprNode::Param(Value::I64(1))),
        Box::new(ExprNode::Param(Value::I64(1))),
    ));
    assert_eq!(
        render_query(&where_(bad_op), &Postgres),
        Err(RenderError::InvalidToken("= 1; --"))
    );
    let bad_fn = bool_expr(ExprNode::Call("f(); DROP", vec![]));
    assert_eq!(
        render_query(&where_(bad_fn), &Postgres),
        Err(RenderError::InvalidToken("f(); DROP"))
    );
    let bad_key = bool_expr(ExprNode::JsonPath {
        column: ColumnRef::new("prefs"),
        path: vec![PathSeg::Key("a\0")],
        as_text: true,
    });
    assert_eq!(
        render_query(&where_(bad_key), &Postgres),
        Err(RenderError::InvalidJsonKey("a\0"))
    );
    assert_eq!(
        render_query(
            &insert::<User>().values(&[] as &[User]).affected(),
            &Postgres
        ),
        Err(RenderError::EmptyInsert)
    );
    assert_eq!(
        render_query(&update::<User>().affected(), &Postgres),
        Err(RenderError::EmptyUpdate)
    );
    assert_eq!(
        render_query(
            &insert::<User>().value(&User::sample()).returning_one(),
            &rupa_core::Memory
        ),
        Err(RenderError::UnsupportedDialect(DialectId::Memory))
    );
    assert_eq!(
        render_query(&select::<User>().all(), &DynDialect::new(DialectId::MySql)),
        Err(RenderError::UnsupportedDialect(DialectId::MySql))
    );
}

#[test]
fn dyn_dialect_renders_like_static() {
    let q = where_(dsl(&ILIKE));
    assert_eq!(
        render_query(&q, &DynDialect::new(DialectId::Postgres)),
        render_query(&q, &Postgres)
    );
}

#[test]
fn raw_statement_ir() {
    let s = Statement::Raw(RawSql {
        parts: vec![RawPart::Sql("SELECT "), RawPart::Param(Value::I32(1))],
    });
    insta::assert_snapshot!(pg_stmt(&s));
}
