use rupa::migrate::Planned;
use rupa::sql::{ConstraintChange, ExprRefusal, SchemaError};
use rupa::{DialectId, Relation};

use super::support::{accounts, add, dialect, planned, refused_at_preflight};
use super::{Harness, Session, catalog};
use crate::migrations::fixture::{
    Account, Author, Plans, add_account_constraints, create_accounts_checked_by_a_foreign_column,
    create_accounts_checked_by_an_aggregate, create_accounts_checked_by_the_identity,
    create_accounts_then_add_a_constraint, create_accounts_then_drop_a_constraint,
    create_accounts_with_a_unique_nickname, create_accounts_with_a_unique_of_a_foreign_column,
    create_accounts_with_constraints, create_plain_accounts, drop_account_constraints,
};

/// Asserts the constraints of the fixtures with rows of `owner`: a bucket
/// of zero violates the check, a repeated email and bucket the unique pair,
/// and a repeated email under another bucket violates neither.
fn assert_enforced<H: Harness>(h: &H, owner: &str, enforced: bool) {
    let email = format!("{owner}@example.com");
    let before = accounts(h);

    assert!(add(h, &email, "a", 1, None));
    assert_eq!(
        add(h, &email, "dup", 1, None),
        !enforced,
        "the unique pair email and bucket"
    );
    assert_eq!(
        add(h, &email, "zero", 0, None),
        !enforced,
        "the positive bucket check"
    );
    assert!(add(h, &email, "other", 2, None));
    assert_eq!(
        accounts(h) - before,
        if enforced { 2 } else { 4 },
        "only the rows the constraints allow are stored"
    );
}

/// A table created with a check and a unique pair rejects violating rows
/// and accepts valid ones on every engine.
pub(super) fn case_created_constraints_enforce_on_every_engine<H: Harness>(h: &mut H) {
    let entries = [Planned(Plans(
        planned::<H>("1"),
        create_accounts_with_constraints,
    ))];

    wait!(h, h.connect().apply(&catalog(&entries))).expect("constrained accounts apply");

    assert_enforced(h, "created", true);
}

/// PostgreSQL and MySQL add constraints to a table and drop them again;
/// SQLite refuses both at preflight, before anything is acquired.
pub(super) fn case_constraints_alter_or_fail_typed_before_acquisition<H: Harness>(h: &mut H) {
    let dialect = dialect(h);

    if dialect == DialectId::Sqlite {
        for (plan, change, constraint) in [
            (
                create_accounts_then_add_a_constraint as fn() -> rupa::migrate::Plan,
                ConstraintChange::Add,
                "accounts_bucket_positive",
            ),
            (
                create_accounts_then_drop_a_constraint,
                ConstraintChange::Drop,
                "accounts_bucket_positive",
            ),
        ] {
            assert_eq!(
                refused_at_preflight(h, plan),
                SchemaError::ConstraintAlteration {
                    constraint,
                    dialect,
                    change,
                }
            );
        }

        return;
    }

    let created = [Planned(Plans(planned::<H>("1"), create_plain_accounts))];
    let added = [
        Planned(Plans(planned::<H>("1"), create_plain_accounts)),
        Planned(Plans(planned::<H>("2"), add_account_constraints)),
    ];
    let dropped = [
        Planned(Plans(planned::<H>("1"), create_plain_accounts)),
        Planned(Plans(planned::<H>("2"), add_account_constraints)),
        Planned(Plans(planned::<H>("3"), drop_account_constraints)),
    ];

    wait!(h, h.connect().apply(&catalog(&created))).expect("plain accounts apply");
    assert!(add(h, "seed@example.com", "seed", 5, None));

    wait!(h, h.connect().apply(&catalog(&added))).expect("constraints are added");
    assert_enforced(h, "added", true);

    wait!(h, h.connect().apply(&catalog(&dropped))).expect("constraints are dropped");
    assert_enforced(h, "dropped", false);
}

/// A unique constraint over unbounded text is built on PostgreSQL and SQLite
/// and enforces there. MySQL cannot, and says so at preflight.
pub(super) fn case_unique_constraints_over_text_work_or_fail_typed_before_acquisition<
    H: Harness,
>(
    h: &mut H,
) {
    if dialect(h) == DialectId::MySql {
        let error = refused_at_preflight(h, create_accounts_with_a_unique_nickname);

        assert!(
            matches!(
                error,
                SchemaError::Unindexable {
                    column: "nick",
                    dialect: DialectId::MySql,
                    ..
                }
            ),
            "{error:?}"
        );

        return;
    }

    let entries = [Planned(Plans(
        planned::<H>("1"),
        create_accounts_with_a_unique_nickname,
    ))];

    wait!(h, h.connect().apply(&catalog(&entries))).expect("a text unique applies");

    assert!(add(h, "a@example.com", "ada", 1, None));
    assert!(!add(h, "b@example.com", "ada", 2, None));
}

/// A check over a column of another table or an aggregate fails typed at
/// preflight on every engine.
pub(super) fn case_checks_ddl_cannot_hold_fail_typed_before_acquisition<H: Harness>(h: &mut H) {
    let dialect = dialect(h);

    for (plan, object, reason) in [
        (
            create_accounts_checked_by_a_foreign_column as fn() -> rupa::migrate::Plan,
            "accounts_foreign",
            ExprRefusal::Column {
                column: "name",
                table: Some(<Author as Relation>::TABLE),
                expected: <Account as Relation>::TABLE,
            },
        ),
        (
            create_accounts_checked_by_an_aggregate,
            "accounts_counted",
            ExprRefusal::Aggregate { function: "count" },
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

/// A unique constraint built from another entity's column is refused at
/// preflight on every engine, even when the table has a column of the same
/// name.
pub(super) fn case_a_unique_of_another_entitys_column_is_refused_before_acquisition<H: Harness>(
    h: &mut H,
) {
    assert_eq!(
        refused_at_preflight(h, create_accounts_with_a_unique_of_a_foreign_column),
        SchemaError::ConstraintTable {
            constraint: "accounts_wrong_table",
            recorded: <Author as Relation>::TABLE,
            expected: <Account as Relation>::TABLE,
        }
    );
}

/// A check over the auto-increment id works on PostgreSQL and SQLite.
/// MySQL refuses it at preflight.
pub(super) fn case_checks_over_the_identity_work_or_fail_typed_before_acquisition<H: Harness>(
    h: &mut H,
) {
    let dialect = dialect(h);

    if dialect == DialectId::MySql {
        assert_eq!(
            refused_at_preflight(h, create_accounts_checked_by_the_identity),
            SchemaError::Expression {
                object: "accounts_id_positive",
                dialect,
                reason: ExprRefusal::AutoIncrementColumn { column: "id" },
            }
        );

        return;
    }

    let entries = [Planned(Plans(
        planned::<H>("1"),
        create_accounts_checked_by_the_identity,
    ))];

    wait!(h, h.connect().apply(&catalog(&entries))).expect("a check over the id applies");

    assert!(add(h, "a@example.com", "a", 1, None));
    assert_eq!(accounts(h), 1);
}
