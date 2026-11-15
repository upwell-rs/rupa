//! Plans that constrain accounts.

use rupa::core::expr::IntoExpr;
use rupa::migrate::Plan;
use rupa::migrate::schema::{Check, ConstraintKind, Table, Unique};
use rupa::{Entity, Expr, ExprOps};

use super::Author;
use super::expressions::{Account, create_accounts};

const BUCKET_POSITIVE: &str = "accounts_bucket_positive";
const EMAIL_BUCKET: &str = "accounts_email_bucket";

fn bucket_positive() -> Check {
    Check::on::<Account>(BUCKET_POSITIVE, Account::BUCKET.gt(0))
}

fn email_bucket() -> Unique {
    Unique::on(EMAIL_BUCKET, Account::EMAIL)
        .and(Account::BUCKET)
        .build()
}

/// Accounts created with a positive-bucket check and a unique pair of email
/// and bucket, which every dialect builds and enforces.
pub fn create_accounts_with_constraints() -> Plan {
    Plan::new().create_table(
        Table::of::<Account>()
            .check(bucket_positive())
            .unique(email_bucket()),
    )
}

/// Accounts without constraints.
pub fn create_plain_accounts() -> Plan {
    create_accounts()
}

/// Adds the check and the unique pair to existing accounts, which
/// PostgreSQL and MySQL do and SQLite refuses.
pub fn add_account_constraints() -> Plan {
    Plan::new()
        .add_constraint(Account::TABLE, bucket_positive())
        .add_constraint(Account::TABLE, email_bucket())
}

/// Drops the check and the unique pair again.
pub fn drop_account_constraints() -> Plan {
    Plan::new()
        .drop_constraint(Account::TABLE, BUCKET_POSITIVE, ConstraintKind::Check)
        .drop_constraint(Account::TABLE, EMAIL_BUCKET, ConstraintKind::Unique)
}

/// Accounts created, then constrained in the same plan.
pub fn create_accounts_then_add_a_constraint() -> Plan {
    create_accounts().add_constraint(Account::TABLE, bucket_positive())
}

/// Accounts created, then a constraint dropped in the same plan.
pub fn create_accounts_then_drop_a_constraint() -> Plan {
    create_accounts().drop_constraint(Account::TABLE, BUCKET_POSITIVE, ConstraintKind::Check)
}

/// A unique constraint over the unbounded nickname, which PostgreSQL and
/// SQLite build and MySQL refuses.
pub fn create_accounts_with_a_unique_nickname() -> Plan {
    Plan::new()
        .create_table(Table::of::<Account>().unique(Unique::on("accounts_nick", Account::NICK)))
}

/// A check over a column of another table.
pub fn create_accounts_checked_by_a_foreign_column() -> Plan {
    Plan::new().create_table(Table::of::<Account>().check(Check::on::<Account>(
        "accounts_foreign",
        Author::NAME.eq("ada".to_owned()),
    )))
}

/// A check over an aggregate.
pub fn create_accounts_checked_by_an_aggregate() -> Plan {
    Plan::new().create_table(
        Table::of::<Account>().check(Check::on::<Account>(
            "accounts_counted",
            Expr::<i64>::call(
                "count",
                [Expr::<i32>::from_node(Account::BUCKET.into_node())],
            )
            .gt(0_i64),
        )),
    )
}
