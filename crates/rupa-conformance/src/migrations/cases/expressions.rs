use rupa::migrate::Planned;
use rupa::sql::{ExprRefusal, IndexRefusal, SchemaError};
use rupa::{DialectId, Entity, VarChar, insert};

use super::support::{dialect, planned, schema_refusal};
use super::{Harness, Session, catalog};
use crate::migrations::fixture::{
    Account, Author, NewAccount, Plans, create_accounts_indexed_by_a_foreign_column,
    create_accounts_indexed_by_a_same_named_foreign_column,
    create_accounts_indexed_by_a_volatile_function, create_accounts_indexed_by_an_aggregate,
    create_accounts_with_expression_indexes, create_accounts_with_float_expression,
    create_accounts_with_partial_index, create_accounts_with_text_expression,
};

/// Inserts one account on its own connection, and whether the database
/// accepted it.
fn add<H: Harness>(h: &H, email: &str, nick: &str, bucket: i32, closed_at: Option<i64>) -> bool {
    let session = h.connect();
    let row = NewAccount {
        email: VarChar::new(email).expect("a short email"),
        nick: nick.to_owned(),
        bucket,
        closed_at,
    };

    wait!(
        h,
        insert::<Account>()
            .values(&[row])
            .affected()
            .execute(&session)
    )
    .is_ok()
}

fn accounts<H: Harness>(h: &H) -> i64 {
    h.count("SELECT count(*) FROM accounts")
}

/// Applies `plan` and asserts that preflight refused it, before anything
/// was acquired or created.
fn refused_at_preflight<H: Harness>(h: &H, plan: fn() -> rupa::migrate::Plan) -> SchemaError {
    let entries = [Planned(Plans(planned::<H>("1"), plan))];
    let error = schema_refusal(wait!(h, h.connect().apply(&catalog(&entries))));

    assert!(!h.table_exists("_rupa_migrations"));
    assert!(!h.table_exists("accounts"));

    error
}

/// A unique index over an expression enforces on every engine: the
/// lowercased email on all of them, and an arithmetic expression whose
/// duplicates the plain column would not collide on.
pub(super) fn case_expression_unique_indexes_enforce_on_every_engine<H: Harness>(h: &mut H) {
    let entries = [Planned(Plans(
        planned::<H>("1"),
        create_accounts_with_expression_indexes,
    ))];

    wait!(h, h.connect().apply(&catalog(&entries))).expect("expression indexes apply");

    assert!(add(h, "Ada@example.com", "ada", 1, None));
    assert!(
        !add(h, "ada@example.com", "ada2", 2, None),
        "emails differing only in case collide under lower(email)"
    );
    assert!(
        !add(h, "grace@example.com", "grace", 11, None),
        "buckets 1 and 11 collide under bucket % 10"
    );
    assert!(add(h, "grace@example.com", "grace", 12, None));
    assert_eq!(accounts(h), 2);
}

/// An expression of unbounded text is indexed on PostgreSQL and SQLite and
/// enforces there. MySQL cannot, and says so at preflight.
pub(super) fn case_text_expression_indexes_work_or_fail_typed_before_acquisition<H: Harness>(
    h: &mut H,
) {
    if dialect(h) == DialectId::MySql {
        let error = refused_at_preflight(h, create_accounts_with_text_expression);

        assert!(
            matches!(
                error,
                SchemaError::Index {
                    index: "accounts_nick_lower",
                    dialect: DialectId::MySql,
                    reason: IndexRefusal::UnboundedExpr { .. },
                }
            ),
            "{error:?}"
        );

        return;
    }

    let entries = [Planned(Plans(
        planned::<H>("1"),
        create_accounts_with_text_expression,
    ))];

    wait!(h, h.connect().apply(&catalog(&entries))).expect("a text expression index applies");

    assert!(add(h, "ada@example.com", "Ada", 1, None));
    assert!(
        !add(h, "grace@example.com", "ada", 2, None),
        "nicknames differing only in case collide under lower(nick)"
    );
    assert_eq!(accounts(h), 1);
}

/// A partial index applies on PostgreSQL and SQLite and only constrains the
/// rows it covers. MySQL has none, and refuses at preflight.
pub(super) fn case_partial_indexes_work_or_fail_typed_before_acquisition<H: Harness>(h: &mut H) {
    if dialect(h) == DialectId::MySql {
        let error = refused_at_preflight(h, create_accounts_with_partial_index);

        assert!(
            matches!(
                error,
                SchemaError::Index {
                    index: "accounts_open_bucket",
                    dialect: DialectId::MySql,
                    reason: IndexRefusal::Partial,
                }
            ),
            "{error:?}"
        );

        return;
    }

    let entries = [Planned(Plans(
        planned::<H>("1"),
        create_accounts_with_partial_index,
    ))];

    wait!(h, h.connect().apply(&catalog(&entries))).expect("a partial index applies");

    assert!(add(h, "a@example.com", "a", 1, None));
    assert!(add(h, "b@example.com", "b", 1, Some(100)));
    assert!(add(h, "c@example.com", "c", 1, Some(200)));
    assert!(
        !add(h, "d@example.com", "d", 1, None),
        "two open accounts share a bucket"
    );
    assert!(add(h, "e@example.com", "e", 2, None));
    assert_eq!(accounts(h), 4);
}

/// What DDL cannot hold fails typed at preflight on every engine, before
/// anything is acquired or created.
pub(super) fn case_expressions_ddl_cannot_hold_fail_typed_before_acquisition<H: Harness>(
    h: &mut H,
) {
    let dialect = dialect(h);

    for (plan, object, reason) in [
        (
            create_accounts_indexed_by_an_aggregate as fn() -> rupa::migrate::Plan,
            "accounts_counted",
            ExprRefusal::Aggregate { function: "count" },
        ),
        (
            create_accounts_indexed_by_a_volatile_function,
            "accounts_random",
            ExprRefusal::Volatile { function: "random" },
        ),
        (
            create_accounts_indexed_by_a_foreign_column,
            "accounts_foreign",
            ExprRefusal::Column {
                column: "name",
                table: Some(<Author as Entity>::TABLE),
                expected: <Account as Entity>::TABLE,
            },
        ),
        (
            create_accounts_indexed_by_a_same_named_foreign_column,
            "accounts_same_name",
            ExprRefusal::Column {
                column: "id",
                table: Some(<Author as Entity>::TABLE),
                expected: <Account as Entity>::TABLE,
            },
        ),
    ] {
        assert_eq!(
            refused_at_preflight(h, plan),
            SchemaError::Expression {
                object,
                dialect,
                reason,
            }
        );
    }
}

/// A float literal stays a float: `bucket / 2.0` keeps buckets 4 and 5
/// apart, as it would not if the literal were written as the integer `2`.
pub(super) fn case_float_literals_in_index_expressions_stay_floats<H: Harness>(h: &mut H) {
    let entries = [Planned(Plans(
        planned::<H>("1"),
        create_accounts_with_float_expression,
    ))];

    wait!(h, h.connect().apply(&catalog(&entries))).expect("a float expression index applies");

    assert!(add(h, "a@example.com", "a", 4, None));
    assert!(add(h, "b@example.com", "b", 5, None));
    assert!(
        !add(h, "c@example.com", "c", 4, None),
        "the same bucket still collides"
    );
    assert_eq!(accounts(h), 2);
}
