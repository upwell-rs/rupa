//! Joins evaluated from hand-built IR (the typed join builder comes later).

use rupa_core::column::{ColumnKind, ColumnMeta};
use rupa_core::exec::Outcome;
use rupa_core::ir::{
    BinOp, ColumnRef, Delete, Direction, ExprNode, FromClause, Insert, Join, JoinKind, OrderBy,
    Select, SourceId, Statement, TableRef,
};
use rupa_core::{SqlType, Value};

use crate::{MemoryDb, MemoryError};

const USERS: TableRef = TableRef::new("users");
const ORDERS: TableRef = TableRef::new("orders");

fn meta(name: &'static str, sql_type: SqlType, nullable: bool) -> ColumnMeta {
    ColumnMeta {
        field: name,
        name,
        kind: ColumnKind::Scalar,
        sql_type,
        nullable,
        generated: false,
    }
}

fn insert(db: &mut MemoryDb, table: TableRef, columns: &[&'static str], rows: Vec<Vec<Value>>) {
    let rows = rows
        .into_iter()
        .map(|r| r.into_iter().map(ExprNode::Param).collect())
        .collect();
    db.run_statement(&Statement::Insert(Insert {
        table,
        columns: columns.to_vec(),
        rows,
        returning: None,
    }))
    .expect("insert");
}

/// users: 1 ann (manager none), 2 bob (manager 1), 3 cy (manager 1)
/// orders: 10 -> ann 5, 11 -> ann 7, 12 -> bob 3
fn db() -> MemoryDb {
    let mut db = MemoryDb::new();
    db.create_table(
        USERS,
        vec![
            meta("id", SqlType::I64, false),
            meta("name", SqlType::Text, false),
            meta("manager_id", SqlType::I64, true),
        ],
        &["id"],
    )
    .create_table(
        ORDERS,
        vec![
            meta("id", SqlType::I64, false),
            meta("user_id", SqlType::I64, false),
            meta("total", SqlType::I64, false),
        ],
        &["id"],
    );
    let text = |s: &str| Value::Text(s.into());
    insert(
        &mut db,
        USERS,
        &["id", "name", "manager_id"],
        vec![
            vec![Value::I64(1), text("ann"), Value::Null(SqlType::I64)],
            vec![Value::I64(2), text("bob"), Value::I64(1)],
            vec![Value::I64(3), text("cy"), Value::I64(1)],
        ],
    );
    insert(
        &mut db,
        ORDERS,
        &["id", "user_id", "total"],
        vec![
            vec![Value::I64(10), Value::I64(1), Value::I64(5)],
            vec![Value::I64(11), Value::I64(1), Value::I64(7)],
            vec![Value::I64(12), Value::I64(2), Value::I64(3)],
        ],
    );
    db
}

fn col(source: u16, name: &'static str) -> ColumnRef {
    ColumnRef::of(SourceId(source), name)
}

fn eq(l: ExprNode, r: ExprNode) -> ExprNode {
    ExprNode::Binary(BinOp::Eq, Box::new(l), Box::new(r))
}

fn c(source: u16, name: &'static str) -> ExprNode {
    ExprNode::Column(col(source, name))
}

/// `users t0 <kind> JOIN orders t1 ON t1.user_id = t0.id`
fn users_orders(kind: JoinKind) -> FromClause {
    FromClause::Join {
        base: USERS,
        joins: vec![Join {
            kind,
            table: ORDERS,
            on: eq(c(1, "user_id"), c(0, "id")),
        }],
    }
}

fn select(from: FromClause, projection: Vec<ColumnRef>) -> Select {
    Select {
        from,
        projection,
        filter: None,
        order_by: vec![],
        limit: None,
        offset: None,
    }
}

fn rows(db: &mut MemoryDb, s: Select) -> Result<Vec<Vec<Value>>, MemoryError> {
    let types: Vec<SqlType> = vec![SqlType::I64; s.projection.len()];
    let Outcome::Rows(mut cursor) = db.run_statement(&Statement::Select(s))? else {
        panic!("a select yields rows");
    };
    let mut out = Vec::new();
    while let Some(row) = cursor.next_row() {
        let row = row.expect("row");
        out.push(
            types
                .iter()
                .enumerate()
                .map(|(i, ty)| row.get(i, *ty).expect("value"))
                .collect(),
        );
    }
    Ok(out)
}

fn ids(rows: &[Vec<Value>]) -> Vec<Vec<Option<i64>>> {
    rows.iter()
        .map(|r| {
            r.iter()
                .map(|v| match v {
                    Value::I64(n) => Some(*n),
                    Value::Null(_) => None,
                    other => panic!("unexpected {other:?}"),
                })
                .collect()
        })
        .collect()
}

#[test]
fn inner_join_pairs_matching_rows() {
    let s = select(
        users_orders(JoinKind::Inner),
        vec![col(0, "id"), col(1, "id")],
    );
    let got = rows(&mut db(), s).unwrap();
    assert_eq!(
        ids(&got),
        [
            [Some(1), Some(10)],
            [Some(1), Some(11)],
            [Some(2), Some(12)]
        ]
    );
}

#[test]
fn left_join_keeps_unmatched_rows_with_typed_nulls() {
    let s = select(
        users_orders(JoinKind::Left),
        vec![col(0, "id"), col(1, "id"), col(1, "total")],
    );
    let got = rows(&mut db(), s).unwrap();
    assert_eq!(
        ids(&got),
        [
            [Some(1), Some(10), Some(5)],
            [Some(1), Some(11), Some(7)],
            [Some(2), Some(12), Some(3)],
            [Some(3), None, None],
        ]
    );
    assert_eq!(got[3][1], Value::Null(SqlType::I64));
}

#[test]
fn filter_and_order_by_on_joined_columns() {
    let mut s = select(
        users_orders(JoinKind::Left),
        vec![col(0, "id"), col(1, "total")],
    );
    // Postgres order: NULLs last ascending, so cy (no orders) comes last.
    s.order_by = vec![OrderBy {
        expr: c(1, "total"),
        direction: Direction::Asc,
    }];
    s.filter = Some(ExprNode::Binary(
        BinOp::Ne,
        Box::new(c(1, "total")),
        Box::new(ExprNode::Param(Value::I64(7))),
    ));
    let got = rows(&mut db(), s).unwrap();
    // `NULL <> 7` is NULL, so the left join's unmatched row is filtered out too.
    assert_eq!(ids(&got), [[Some(2), Some(3)], [Some(1), Some(5)]]);
}

#[test]
fn self_join_through_two_sources_of_one_table() {
    let from = FromClause::Join {
        base: USERS,
        joins: vec![Join {
            kind: JoinKind::Inner,
            table: USERS,
            on: eq(c(1, "id"), c(0, "manager_id")),
        }],
    };
    let got = rows(&mut db(), select(from, vec![col(0, "id"), col(1, "id")])).unwrap();
    assert_eq!(ids(&got), [[Some(2), Some(1)], [Some(3), Some(1)]]);
}

#[test]
fn three_way_join_may_refer_to_any_earlier_source() {
    // users t0 JOIN orders t1 ON t1.user_id = t0.id
    //          LEFT JOIN users t2 ON t2.id = t0.manager_id
    let FromClause::Join { base, mut joins } = users_orders(JoinKind::Inner) else {
        unreachable!()
    };
    joins.push(Join {
        kind: JoinKind::Left,
        table: USERS,
        on: eq(c(2, "id"), c(0, "manager_id")),
    });
    let from = FromClause::Join { base, joins };
    let got = rows(&mut db(), select(from, vec![col(1, "id"), col(2, "id")])).unwrap();
    assert_eq!(
        ids(&got),
        [[Some(10), None], [Some(11), None], [Some(12), Some(1)]]
    );
}

#[test]
fn exists_over_a_join() {
    let mut s = select(users_orders(JoinKind::Inner), vec![]);
    s.filter = Some(eq(c(0, "id"), ExprNode::Param(Value::I64(3))));
    let Outcome::Rows(mut cursor) = db().run_statement(&Statement::Exists(s)).unwrap() else {
        panic!("exists yields a row");
    };
    let row = cursor.next_row().unwrap().unwrap();
    assert_eq!(row.get(0, SqlType::Bool).unwrap(), Value::Bool(false));
}

#[test]
fn unknown_sources_are_errors() {
    let s = select(FromClause::Table(USERS), vec![col(1, "id")]);
    assert_eq!(
        rows(&mut db(), s).unwrap_err(),
        MemoryError::NoSuchSource(SourceId(1))
    );
    let err = db()
        .run_statement(&Statement::Delete(Delete {
            table: USERS,
            filter: Some(eq(c(1, "id"), ExprNode::Param(Value::I64(1)))),
        }))
        .err()
        .unwrap();
    assert_eq!(err, MemoryError::NoSuchSource(SourceId(1)));
}
