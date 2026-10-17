//! `#[column(references = ..)]`: the `ON` conditions that inferred joins render.

use rupa::core::Postgres;
use rupa::prelude::*;

#[derive(Debug, Clone, PartialEq, Entity)]
#[entity(table = "users")]
pub struct User {
    #[id]
    id: i64,
    #[column(references = User, relation = manager)]
    manager_id: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Entity)]
#[entity(table = "orders")]
pub struct Order {
    #[id]
    id: i64,
    #[column(references = User)]
    user_id: i64,
    #[column(name = "reviewer", references = User, relation = reviewer)]
    reviewed_by: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Entity)]
#[entity(table = "lines")]
pub struct Line {
    #[id]
    id: i64,
    #[column(references = Order)]
    order_id: i64,
}

fn sql<R>(q: &Query<R>) -> String {
    rupa_sql::render_query(q, &Postgres).expect("render").sql
}

/// Only the part after `FROM`, which is what relations decide.
fn from<R>(q: &Query<R>) -> String {
    let sql = sql(q);
    sql[sql.find(" FROM ").expect("a select") + 1..].to_owned()
}

#[test]
fn a_unique_relation_is_inferred_in_either_direction() {
    // Line -> Order: the joined entity holds the reference.
    assert_eq!(
        from(&select::<Order>().join::<Line>().inner().all()),
        r#"FROM "orders" AS "t0" INNER JOIN "lines" AS "t1" ON "t1"."order_id" = "t0"."id""#
    );
    // Order <- Line: the joined entity is referenced.
    assert_eq!(
        from(&select::<Line>().join::<Order>().left().all()),
        r#"FROM "lines" AS "t0" LEFT JOIN "orders" AS "t1" ON "t0"."order_id" = "t1"."id""#
    );
}

#[test]
fn relations_to_later_sources_are_found_by_type() {
    // Order has two relations to User, so pick one; then Line finds Order at
    // position 1.
    let q = select::<User>()
        .join::<Order>()
        .inner_via(Order::reviewer())
        .join::<Line>()
        .left()
        .all();
    assert_eq!(
        from(&q),
        concat!(
            r#"FROM "users" AS "t0" INNER JOIN "orders" AS "t1" ON "t1"."reviewer" = "t0"."id" "#,
            r#"LEFT JOIN "lines" AS "t2" ON "t2"."order_id" = "t1"."id""#,
        )
    );
    let _: &Query<Vec<(User, Order, Option<Line>)>> = &q;
    // The default accessor name drops `_id`.
    assert_eq!(
        from(
            &select::<User>()
                .join::<Order>()
                .inner_via(Order::user())
                .all()
        ),
        r#"FROM "users" AS "t0" INNER JOIN "orders" AS "t1" ON "t1"."user_id" = "t0"."id""#
    );
}

#[test]
fn a_self_reference_needs_its_relation_and_direction() {
    // `manager` fits both ways round (an employee's manager, a manager's
    // reports), so even `via` is ambiguous: the condition is written out.
    let q = select::<User>()
        .left_join_on::<User>(|(employee, manager)| {
            manager
                .col(col!(User::id))
                .eq(employee.col(col!(User::manager_id)))
        })
        .all();
    assert!(from(&q).ends_with(r#"ON "t1"."id" = "t0"."manager_id""#));
}

#[derive(Debug, PartialEq, Joined)]
pub struct LineView {
    #[from(Line::id)]
    line: i64,
    #[from(User::id)]
    customer: i64,
    #[flatten]
    order: Order,
}

#[test]
fn a_joined_struct_selects_only_its_columns() {
    let q = select::<Line>()
        .join::<Order>()
        .inner()
        .join::<User>()
        .inner_via(Order::user())
        .into::<LineView>()
        .all();
    let sql = sql(&q);
    assert!(
        sql.starts_with(
            r#"SELECT "t0"."id", "t2"."id", "t1"."id", "t1"."user_id", "t1"."reviewer" FROM "lines" AS "t0""#
        ),
        "{sql}"
    );
}
