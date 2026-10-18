//! Join rendering from hand-built IR, one snapshot per dialect, plus the
//! render cache over join shapes.

use std::fmt::Write as _;

use rupa_core::ir::{
    BinOp, ColumnRef, Direction, DslCall, DslFnDef, ExprNode, FromClause, Join, JoinKind, OrderBy,
    PathSeg, Select, SourceId, Statement, TableRef, Update,
};
use rupa_core::{Dialect, DslError, MySql, Postgres, Sqlite, Value};
use rupa_sql::{RenderCache, RenderError, render};

const USERS: TableRef = TableRef::with_schema("app", "users");
const ORDERS: TableRef = TableRef::new("orders");

fn c(source: u16, name: &'static str) -> ExprNode {
    ExprNode::Column(ColumnRef::of(SourceId(source), name))
}

fn bin(op: BinOp, l: ExprNode, r: ExprNode) -> ExprNode {
    ExprNode::Binary(op, Box::new(l), Box::new(r))
}

fn param(n: i64) -> ExprNode {
    ExprNode::Param(Value::I64(n))
}

/// `users t0 INNER JOIN orders t1 ON t1.user_id = t0.id AND t1.total > $min
///  LEFT JOIN users t2 ON t2.id = t0.manager_id`
fn joined(min: i64) -> Select {
    Select {
        from: FromClause::Join {
            base: USERS,
            joins: vec![
                Join {
                    kind: JoinKind::Inner,
                    table: ORDERS,
                    on: bin(
                        BinOp::And,
                        bin(BinOp::Eq, c(1, "user_id"), c(0, "id")),
                        bin(BinOp::Gt, c(1, "total"), param(min)),
                    ),
                },
                Join {
                    kind: JoinKind::Left,
                    table: USERS,
                    on: bin(BinOp::Eq, c(2, "id"), c(0, "manager_id")),
                },
            ],
        },
        projection: vec![
            ColumnRef::of(SourceId(0), "id"),
            ColumnRef::of(SourceId(1), "id"),
            ColumnRef::of(SourceId(2), "name"),
        ],
        filter: Some(bin(
            BinOp::Eq,
            ExprNode::JsonPath {
                column: ColumnRef::of(SourceId(0), "prefs"),
                path: vec![PathSeg::Key("theme")],
                as_text: true,
            },
            ExprNode::Param(Value::Text("dark".into())),
        )),
        order_by: vec![OrderBy {
            expr: c(2, "name"),
            direction: Direction::Asc,
        }],
        limit: Some(param(10)),
        offset: None,
    }
}

fn show(s: &Statement, d: &dyn Dialect) -> String {
    match render(s, d) {
        Ok(r) => {
            let mut out = r.sql;
            for (i, p) in r.params.iter().enumerate() {
                let _ = write!(out, "\n  {} = {p:?}", i + 1);
            }
            out
        }
        Err(e) => format!("error: {e:?}"),
    }
}

fn cases(d: &dyn Dialect) -> String {
    let single = Select {
        from: FromClause::Table(USERS),
        projection: vec![ColumnRef::new("id")],
        filter: None,
        order_by: vec![],
        limit: None,
        offset: None,
    };
    let all: Vec<(&str, String)> = vec![
        ("select", show(&Statement::Select(joined(5)), d)),
        ("exists", show(&Statement::Exists(joined(5)), d)),
        (
            "single table stays unqualified",
            show(&Statement::Select(single.clone()), d),
        ),
        (
            "unknown source in a single-table select",
            show(
                &Statement::Select(Select {
                    projection: vec![ColumnRef::of(SourceId(1), "id")],
                    ..single
                }),
                d,
            ),
        ),
        (
            "unknown source in an update filter",
            show(
                &Statement::Update(Update {
                    table: USERS,
                    set: vec![("name", ExprNode::Param(Value::Text("x".into())))],
                    filter: Some(bin(BinOp::Eq, c(1, "id"), param(1))),
                }),
                d,
            ),
        ),
    ];
    all.into_iter()
        .map(|(name, sql)| format!("-- {name}\n{sql}\n"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn postgres() {
    insta::assert_snapshot!(cases(&Postgres));
}

#[test]
fn mysql() {
    insta::assert_snapshot!(cases(&MySql));
}

#[test]
fn sqlite() {
    insta::assert_snapshot!(cases(&Sqlite));
}

#[test]
fn join_shapes_are_cached_and_rebound() {
    let cache = RenderCache::new(1024);
    let first = cache
        .render(&Statement::Select(joined(5)), &Postgres)
        .unwrap();
    let hit = cache
        .render(&Statement::Select(joined(9)), &Postgres)
        .unwrap();
    assert_eq!(first.sql, hit.sql);
    // ON parameters come before WHERE and LIMIT parameters.
    assert_eq!(hit.params[0], Value::I64(9));
    assert_eq!(cache.stats().hits, 1);

    // A different join kind is a different shape.
    let mut left = joined(5);
    if let FromClause::Join { joins, .. } = &mut left.from {
        joins[0].kind = JoinKind::Left;
    }
    let other = cache.render(&Statement::Select(left), &Postgres).unwrap();
    assert_ne!(other.sql, first.sql);
    assert_eq!(cache.stats().hits, 1);
}

#[test]
fn unknown_source_error_names_the_source() {
    let err = render(
        &Statement::Update(Update {
            table: USERS,
            set: vec![("name", param(1))],
            filter: Some(bin(BinOp::Eq, c(2, "id"), param(1))),
        }),
        &Postgres,
    )
    .unwrap_err();
    assert_eq!(err, RenderError::UnknownSource(SourceId(2)));
}

fn lower_same(_: &dyn Dialect, mut args: Vec<ExprNode>) -> Result<ExprNode, DslError> {
    let r = args.pop().unwrap();
    let l = args.pop().unwrap();
    Ok(bin(BinOp::Eq, l, r))
}
static SAME: DslFnDef = DslFnDef {
    name: "same",
    lower: lower_same,
    eval: None,
};

/// DSL calls in `ON` are lowered before the shape is taken, like everywhere else.
#[test]
fn dsl_calls_in_join_conditions_are_lowered() {
    let mut s = joined(5);
    if let FromClause::Join { joins, .. } = &mut s.from {
        joins[1].on = ExprNode::Dsl(DslCall {
            def: &SAME,
            args: vec![c(2, "id"), c(0, "manager_id")],
        });
    }
    let cache = RenderCache::new(1024);
    let cached = cache
        .render(&Statement::Select(s.clone()), &Postgres)
        .unwrap();
    let direct = render(&Statement::Select(s), &Postgres).unwrap();
    assert_eq!(*cached.sql, direct.sql);
    assert!(
        direct.sql.contains(r#"ON "t2"."id" = "t0"."manager_id""#),
        "{}",
        direct.sql
    );
}

/// A join condition can't use a source joined after it: that alias isn't in
/// scope yet, so it's a render error rather than a database error.
#[test]
fn a_condition_on_a_later_source_is_a_render_error() {
    let mut s = joined(5);
    if let FromClause::Join { joins, .. } = &mut s.from {
        joins[0].on = bin(BinOp::Eq, c(2, "id"), c(0, "id"));
    }
    let err = render(&Statement::Select(s), &Postgres).unwrap_err();
    assert_eq!(err, RenderError::UnknownSource(SourceId(2)));
}
