//! A column's recorded table never changes a statement: the same SQL, the
//! same statement shape and so the same render-cache entry.

use std::sync::Arc;

use rupa_core::Value;
use rupa_core::dialect::{MySql, Postgres, Sqlite};
use rupa_core::ir::{BinOp, ColumnRef, ExprNode, FromClause, Select, Statement, TableRef};
use rupa_sql::{RenderCache, render};

const USERS: TableRef = TableRef::new("users");

fn by_email(column: ColumnRef, email: &str) -> Statement {
    Statement::Select(Select {
        from: FromClause::Table(USERS),
        projection: vec![ColumnRef::new("id")],
        filter: Some(ExprNode::Binary(
            BinOp::Eq,
            Box::new(ExprNode::Column(column)),
            Box::new(ExprNode::Param(Value::Text(email.into()))),
        )),
        order_by: Vec::new(),
        limit: None,
        offset: None,
    })
}

#[test]
fn a_recorded_table_renders_the_same_sql_on_every_dialect() {
    for dialect in [&Postgres as &dyn rupa_core::Dialect, &MySql, &Sqlite] {
        let plain = render(&by_email(ColumnRef::new("email"), "a"), dialect).expect("renders");
        let tabled = render(
            &by_email(ColumnRef::new("email").in_table(USERS), "a"),
            dialect,
        )
        .expect("renders");

        assert_eq!(plain, tabled);
    }
}

#[test]
fn a_recorded_table_shares_the_render_cache_entry() {
    let cache = RenderCache::new(8);
    let plain = cache
        .render(&by_email(ColumnRef::new("email"), "a"), &Postgres)
        .expect("renders");
    let tabled = cache
        .render(
            &by_email(ColumnRef::new("email").in_table(USERS), "b"),
            &Postgres,
        )
        .expect("renders");
    let elsewhere = cache
        .render(
            &by_email(
                ColumnRef::new("email").in_table(TableRef::with_schema("other", "accounts")),
                "c",
            ),
            &Postgres,
        )
        .expect("renders");
    let stats = cache.stats();

    assert!(Arc::ptr_eq(&plain.sql, &tabled.sql), "one shared text");
    assert!(Arc::ptr_eq(&plain.sql, &elsewhere.sql), "one shared text");
    assert_eq!((stats.hits, stats.misses), (2, 1));
    assert_eq!(tabled.params, [Value::Text("b".into())]);
}
