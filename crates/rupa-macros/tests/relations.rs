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
            manager.col(User::ID).eq(employee.col(User::MANAGER_ID))
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

// `#[query]` with joins.

#[query(
    left_join = Order on Order::user_id == User::id && Order::id > $min,
    filter = User::manager_id is_null,
    order_by = Order::id desc,
    limit = 10,
)]
fn with_orders(min: i64) -> Vec<(User, Option<Order>)>;

#[query(inner_join = Order via Order::reviewer, inner_join = Line, order_by = Line::id)]
fn reviewed_lines() -> Vec<(User, Order, Line)>;

#[query(entity = Line, inner_join = Order, inner_join = User via Order::user)]
fn line_views() -> Vec<LineView>;

#[query(entity = User, inner_join = Order via Order::user, filter = Order::id == $id)]
fn has_order(id: i64) -> bool;

#[test]
fn queries_with_joins() {
    assert_eq!(
        from(&with_orders(5)),
        concat!(
            r#"FROM "users" AS "t0" LEFT JOIN "orders" AS "t1" ON "t1"."user_id" = "t0"."id" AND "t1"."id" > $1 "#,
            r#"WHERE "t0"."manager_id" IS NULL ORDER BY "t1"."id" DESC LIMIT $2"#,
        )
    );
    assert_eq!(
        from(&reviewed_lines()),
        concat!(
            r#"FROM "users" AS "t0" INNER JOIN "orders" AS "t1" ON "t1"."reviewer" = "t0"."id" "#,
            r#"INNER JOIN "lines" AS "t2" ON "t2"."order_id" = "t1"."id" ORDER BY "t2"."id" ASC"#,
        )
    );
    assert!(
        sql(&line_views()).starts_with(r#"SELECT "t0"."id", "t2"."id", "t1"."id""#),
        "{}",
        sql(&line_views())
    );
    assert_eq!(
        sql(&has_order(3)),
        concat!(
            r#"SELECT EXISTS (SELECT 1 FROM "users" AS "t0" INNER JOIN "orders" AS "t1" "#,
            r#"ON "t1"."user_id" = "t0"."id" WHERE "t1"."id" = $1)"#,
        )
    );
}

// Self-joins: aliases in `#[query]`, pinned sources in `Joined`.

#[query(
    entity = User as employee,
    left_join = User as manager on manager::id == employee::manager_id,
    filter = manager::manager_id is_null,
    order_by = employee::id,
)]
fn with_managers() -> Vec<(User, Option<User>)>;

/// Unaliased, `User` is the one source not given an alias.
#[query(inner_join = User as manager on manager::id == User::manager_id)]
fn managed() -> Vec<(User, User)>;

#[derive(Debug, PartialEq, Joined)]
pub struct Reporting {
    #[from(User::id, source = 0)]
    employee: i64,
    #[from(User::id, source = 1)]
    manager: Option<i64>,
}

#[query(entity = User, left_join = User as boss on boss::id == User::manager_id)]
fn reporting() -> Vec<Reporting>;

#[test]
fn self_joins() {
    assert_eq!(
        from(&with_managers()),
        concat!(
            r#"FROM "users" AS "t0" LEFT JOIN "users" AS "t1" ON "t1"."id" = "t0"."manager_id" "#,
            r#"WHERE "t1"."manager_id" IS NULL ORDER BY "t0"."id" ASC"#,
        )
    );
    assert!(from(&managed()).ends_with(r#"ON "t1"."id" = "t0"."manager_id""#));
    assert!(
        sql(&reporting()).starts_with(r#"SELECT "t0"."id", "t1"."id" FROM"#),
        "{}",
        sql(&reporting())
    );
}

/// Schema facts are constants: usable in `const` items, not only at run time.
const ORDER_COLUMNS: &[rupa::core::ColumnMeta] = <Order as rupa::core::Entity>::COLUMNS;

#[test]
fn references_are_recorded_in_column_metadata() {
    let reference = |name: &str| {
        ORDER_COLUMNS
            .iter()
            .find(|c| c.name == name)
            .expect("an order column")
            .references
    };
    let users = rupa::core::Reference {
        table: rupa::core::ir::TableRef::new("users"),
        columns: &["id"],
    };

    assert_eq!(reference("user_id"), Some(users));
    assert_eq!(reference("reviewer"), Some(users));
    assert_eq!(reference("id"), None);
    assert!(
        <User as rupa::core::Entity>::COLUMNS
            .iter()
            .any(|c| c.name == "manager_id" && c.references == Some(users)),
        "a self-reference resolves to the entity's own table"
    );
}
