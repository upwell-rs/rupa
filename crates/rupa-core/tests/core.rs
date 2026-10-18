mod fixtures;

use fixtures::{Prefs, User};
use rupa_core::dialect::{
    Capability, DialectId, DynDialect, Postgres, Supports, capabilities_of, caps,
};
use rupa_core::ir::{BinOp, ColumnRef, ExprNode, FromClause, SourceId, Statement};
use rupa_core::prelude::*;
use rupa_core::query::Output;
use rupa_core::row::ValueRows;
use rupa_core::{ColumnKind, Dialect, Expect, ResultError, SqlType, Src, Value};

fn rows(rows: Vec<Vec<Value>>) -> ValueRows {
    ValueRows::new(rows)
}

fn decode<R: rupa_core::QueryResult>(
    q: &Query<R>,
    mut cursor: ValueRows,
) -> Result<R, ResultError> {
    q.decode(Output::Rows(&mut cursor))
}

// --- builder -> IR ----------------------------------------------------------

#[test]
fn select_projects_entity_columns_in_order() {
    let q = select::<User>().all();
    let Statement::Select(s) = q.statement() else {
        panic!()
    };
    assert_eq!(s.from, FromClause::Table(User::TABLE));
    let names: Vec<_> = s.projection.iter().map(|c| c.name).collect();
    assert_eq!(
        names,
        [
            "id",
            "email",
            "nickname",
            "active",
            "created",
            "prefs",
            "old_prefs"
        ]
    );
    assert_eq!(q.expect(), Expect::Many);
}

#[test]
fn repeated_filters_are_anded_and_values_become_params() {
    let q = select::<User>()
        .filter(User::EMAIL.eq("a@b.c"))
        .filter(User::ACTIVE.eq(true))
        .one();
    let Statement::Select(s) = q.statement() else {
        panic!()
    };
    let expected = ExprNode::Binary(
        BinOp::And,
        Box::new(ExprNode::Binary(
            BinOp::Eq,
            Box::new(ExprNode::Column(ColumnRef::new("email"))),
            Box::new(ExprNode::Param(Value::Text("a@b.c".into()))),
        )),
        Box::new(ExprNode::Binary(
            BinOp::Eq,
            Box::new(ExprNode::Column(ColumnRef::new("active"))),
            Box::new(ExprNode::Param(Value::Bool(true))),
        )),
    );
    assert_eq!(s.filter.as_ref(), Some(&expected));
    assert_eq!(q.expect(), Expect::ExactlyOne);
}

#[test]
fn nullable_columns_compare_against_base_values() {
    // `nickname: Option<String>` compares with `&str`, and with another String column.
    let _ = User::NICKNAME.eq("x");
    let _ = User::NICKNAME.eq(User::EMAIL);
    let _ = User::PREFS.path("theme").text().eq("dark");
}

#[test]
fn exists_drops_ordering_and_paging() {
    let q = select::<User>().order_by(User::ID.asc()).limit(1).exists();
    let Statement::Exists(s) = q.statement() else {
        panic!()
    };
    assert!(s.order_by.is_empty() && s.limit.is_none());
    assert_eq!(q.expect(), Expect::Exists);
}

#[test]
fn insert_encodes_entity_as_stored() {
    let q = insert::<User>().value(&User::sample()).affected();
    let Statement::Insert(i) = q.statement() else {
        panic!()
    };
    let row = &i.rows[0];
    assert_eq!(row[2], ExprNode::Param(Value::Null(SqlType::Text)));
    assert_eq!(
        row[5],
        ExprNode::Param(Value::Json(
            serde_json::json!({"theme": "dark", "beta": true})
        ))
    );
    assert_eq!(
        row[6],
        ExprNode::Param(Value::Null(SqlType::Json)),
        "None JSON is SQL NULL"
    );
}

#[test]
fn update_set_uses_column_codec() {
    let prefs = Prefs {
        theme: "light".into(),
        beta: false,
    };
    let q = update::<User>()
        .set(User::PREFS, &prefs)
        .filter(User::ID.eq(7))
        .affected();
    let Statement::Update(u) = q.statement() else {
        panic!()
    };
    assert_eq!(
        u.set[0],
        (
            "prefs",
            ExprNode::Param(Value::Json(
                serde_json::json!({"theme": "light", "beta": false})
            ))
        )
    );
}

// --- result shapes ----------------------------------------------------------

fn sample_row() -> Vec<Value> {
    User::sample().to_values()
}

#[test]
fn rows_decode_into_each_shape() {
    let u = User::sample();
    assert_eq!(
        decode(
            &select::<User>().all(),
            rows(vec![sample_row(), sample_row()])
        )
        .unwrap(),
        vec![u.clone(), u.clone()]
    );
    assert_eq!(
        decode(&select::<User>().all(), rows(vec![])).unwrap(),
        vec![]
    );
    assert_eq!(
        decode(&select::<User>().optional(), rows(vec![])).unwrap(),
        None
    );
    assert_eq!(
        decode(&select::<User>().optional(), rows(vec![sample_row()])).unwrap(),
        Some(u.clone())
    );
    assert_eq!(
        decode(&select::<User>().one(), rows(vec![sample_row()])).unwrap(),
        u
    );
}

#[test]
fn single_row_shapes_reject_wrong_counts() {
    assert_eq!(
        decode(&select::<User>().one(), rows(vec![])),
        Err(ResultError::NotFound)
    );
    assert_eq!(
        decode(
            &select::<User>().one(),
            rows(vec![sample_row(), sample_row()])
        ),
        Err(ResultError::TooManyRows)
    );
    assert_eq!(
        decode(
            &select::<User>().optional(),
            rows(vec![sample_row(), sample_row()])
        ),
        Err(ResultError::TooManyRows)
    );
}

#[test]
fn affected_and_exists() {
    let del = delete::<User>().filter(User::ID.eq(1));
    assert_eq!(del.clone().affected().decode(Output::Affected(3)), Ok(3));
    assert_eq!(
        del.clone().build::<bool>().decode(Output::Affected(0)),
        Ok(false)
    );
    assert_eq!(
        del.affected().decode(Output::Rows(&mut rows(vec![]))),
        Err(ResultError::ShapeMismatch)
    );
    assert_eq!(
        decode(
            &select::<User>().exists(),
            rows(vec![vec![Value::Bool(true)]])
        ),
        Ok(true)
    );
    assert_eq!(
        select::<User>().all().decode(Output::Affected(1)),
        Err(ResultError::ShapeMismatch)
    );
}

#[test]
fn decode_errors_name_the_column() {
    let mut row = sample_row();
    row[1] = Value::I64(1);
    let Err(ResultError::Decode(e)) = decode(&select::<User>().one(), rows(vec![row])) else {
        panic!()
    };
    assert_eq!(e.column, Some("email"));
}

// --- column kinds via col! (simulated derive output) ------------------------

#[test]
fn col_macro_infers_kinds() {
    assert_eq!(col!(User::email).kind(), ColumnKind::Scalar);
    assert_eq!(col!(User::nickname).kind(), ColumnKind::Scalar);
    assert_eq!(col!(User::prefs).kind(), ColumnKind::Json);
    assert_eq!(col!(User::old_prefs).kind(), ColumnKind::Json);
    assert_eq!(
        col!(User::old_prefs).encode(&None),
        Value::Null(SqlType::Json)
    );
    // Inferred handles are interchangeable with hand-written ones.
    let _: Column<User, Prefs, Json> = col!(User::prefs);
    let _ = select::<User>()
        .filter(col!(User::email).eq("x"))
        .filter(col!(User::prefs).path("beta").cast::<bool>().eq(true));
}

// --- dialect capabilities ---------------------------------------------------

fn needs_ilike<D: Supports<caps::Ilike>>(d: &D) -> DialectId {
    d.id()
}

#[test]
fn static_and_runtime_capabilities_agree() {
    assert_eq!(needs_ilike(&Postgres), DialectId::Postgres);
    for cap in Postgres::CAPABILITIES {
        assert!(Postgres.supports(*cap));
        assert!(DynDialect::new(DialectId::Postgres).supports(*cap));
    }

    assert_eq!(capabilities_of(DialectId::Postgres), Postgres::CAPABILITIES);
    assert!(!DynDialect::new(DialectId::MySql).supports(Capability::Ilike));
    let dynamic: &dyn Dialect = &DynDialect::new(DialectId::Postgres);
    assert_eq!(dynamic.id(), DialectId::Postgres);
}

#[test]
fn ui() {
    trybuild::TestCases::new().compile_fail("tests/ui/*.rs");
}

// --- joins ------------------------------------------------------------------

#[test]
fn join_numbers_sources_and_qualifies_columns() {
    let q = select::<User>()
        .filter(User::ACTIVE.eq(true))
        .left_join_on::<User>(|(a, b)| b.col(User::ID).eq(a.col(User::ID)))
        .filter(|(_, b)| b.col(User::EMAIL).eq("x"))
        .all();
    let Statement::Select(s) = q.statement() else {
        panic!()
    };
    let FromClause::Join { base, joins } = &s.from else {
        panic!("a join")
    };
    assert_eq!(*base, User::TABLE);
    assert_eq!(joins.len(), 1);
    assert_eq!(joins[0].kind, rupa_core::ir::JoinKind::Left);
    // Both sources' columns, in order, each naming its source.
    assert_eq!(s.projection.len(), 2 * User::columns().len());
    assert_eq!(s.projection[0], ColumnRef::new("id"));
    assert_eq!(
        s.projection[User::columns().len()],
        ColumnRef::of(SourceId(1), "id")
    );
    // The filter before the join (plain columns) is on source 0.
    let Some(ExprNode::Binary(BinOp::And, first, second)) = &s.filter else {
        panic!("two ANDed filters")
    };
    assert_eq!(
        **first,
        ExprNode::Binary(
            BinOp::Eq,
            Box::new(ExprNode::Column(ColumnRef::new("active"))),
            Box::new(ExprNode::Param(Value::Bool(true)))
        )
    );
    assert!(matches!(&**second, ExprNode::Binary(BinOp::Eq, l, _)
        if **l == ExprNode::Column(ColumnRef::of(SourceId(1), "email"))));
}

#[test]
fn joined_rows_decode_into_tuples_with_none_for_unmatched() {
    let q = select::<User>()
        .left_join_on::<User>(|(a, b)| b.col(User::ID).eq(a.col(User::ID)))
        .all();
    let user = User::sample();
    let width = User::columns().len();
    let unmatched: Vec<Value> = User::columns()
        .iter()
        .map(|c| Value::Null(c.sql_type))
        .collect();
    let matched = [user.to_values(), user.to_values()].concat();
    let missing = [user.to_values(), unmatched].concat();
    assert_eq!(missing.len(), 2 * width);
    let got = decode(&q, rows(vec![matched, missing])).unwrap();
    assert_eq!(
        got,
        [(user.clone(), Some(user.clone())), (user.clone(), None)]
    );
}

#[test]
fn up_to_ten_sources() {
    let on = |n: i64| move |a: Src<User, 0>| a.col(User::ID).eq(n);
    let q = select::<User>()
        .inner_join_on::<User>(|(a, ..)| on(1)(a))
        .inner_join_on::<User>(|(a, ..)| on(2)(a))
        .inner_join_on::<User>(|(a, ..)| on(3)(a))
        .inner_join_on::<User>(|(a, ..)| on(4)(a))
        .inner_join_on::<User>(|(a, ..)| on(5)(a))
        .inner_join_on::<User>(|(a, ..)| on(6)(a))
        .inner_join_on::<User>(|(a, ..)| on(7)(a))
        .inner_join_on::<User>(|(a, ..)| on(8)(a))
        .left_join_on::<User>(|(a, _, _, _, _, _, _, _, _, j)| on(9)(a).and(j.col(User::ACTIVE)))
        .filter(|(_, _, _, _, _, _, _, _, _, j)| j.col(User::ID).eq(1))
        .all();
    let Statement::Select(s) = q.statement() else {
        panic!()
    };
    assert_eq!(s.from.source_count(), 10);
    assert_eq!(s.projection.len(), 10 * User::columns().len());
    type Ten = (
        User,
        User,
        User,
        User,
        User,
        User,
        User,
        User,
        User,
        Option<User>,
    );
    let _: &Query<Vec<Ten>> = &q;
}
