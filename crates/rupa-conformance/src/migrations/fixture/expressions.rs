//! Accounts and the plans that index expressions of them.

use rupa::VarChar;
use rupa::core::expr::IntoExpr;
use rupa::dsl_std::lower;
use rupa::migrate::Plan;
use rupa::migrate::schema::{BTree, IndexKind, Table};
use rupa::{Expr, ExprOps, bind};

use super::Author;

/// An account: a bounded email, an unbounded nickname, a score bucket and
/// the time it was closed, if it was.
#[derive(Debug, rupa::Entity)]
#[entity(table = "accounts")]
pub struct Account {
    #[id(generated)]
    pub id: i64,
    pub email: VarChar<120>,
    pub nick: String,
    pub bucket: i32,
    pub closed_at: Option<i64>,
}

/// Typed insert input for [`Account`]: no id.
#[derive(Debug, Clone, rupa::Insertable)]
#[insertable(entity = Account)]
pub struct NewAccount {
    pub email: VarChar<120>,
    pub nick: String,
    pub bucket: i32,
    pub closed_at: Option<i64>,
}

fn create_accounts() -> Plan {
    Plan::new().create_table(Table::of::<Account>())
}

fn lower_email() -> Expr<VarChar<120>> {
    Expr::<VarChar<120>>::call(
        "lower",
        [Expr::<VarChar<120>>::from_node(Account::EMAIL.into_node())],
    )
}

fn bucket_of_ten() -> Expr<i32> {
    Expr::<i32>::raw_op(
        "%",
        Expr::<i32>::from_node(Account::BUCKET.into_node()),
        bind(10_i32),
    )
}

/// Accounts with a unique index over the lowercased bounded email and one
/// over the bucket modulo ten, which every dialect builds and enforces.
pub fn create_accounts_with_expression_indexes() -> Plan {
    create_accounts()
        .create_index(BTree::on_expr::<Account>("accounts_email_lower", lower_email()).unique())
        .create_index(BTree::on_expr::<Account>("accounts_bucket_ten", bucket_of_ten()).unique())
}

/// Accounts with a unique index over the lowercased unbounded nickname,
/// which PostgreSQL and SQLite build and MySQL refuses.
pub fn create_accounts_with_text_expression() -> Plan {
    create_accounts().create_index(
        BTree::on_expr::<Account>("accounts_nick_lower", lower(Account::NICK)).unique(),
    )
}

/// Accounts with a unique index over the bucket of the accounts not yet
/// closed, which PostgreSQL and SQLite build and MySQL refuses.
pub fn create_accounts_with_partial_index() -> Plan {
    create_accounts().create_index(
        BTree::on("accounts_open_bucket", Account::BUCKET)
            .unique()
            .filter(Account::CLOSED_AT.is_null()),
    )
}

/// An index over an aggregate.
pub fn create_accounts_indexed_by_an_aggregate() -> Plan {
    create_accounts().create_index(BTree::on_expr::<Account>(
        "accounts_counted",
        Expr::<i64>::call(
            "count",
            [Expr::<i32>::from_node(Account::BUCKET.into_node())],
        ),
    ))
}

/// An index over a volatile function.
pub fn create_accounts_indexed_by_a_volatile_function() -> Plan {
    create_accounts().create_index(BTree::on_expr::<Account>(
        "accounts_random",
        Expr::<f64>::call::<f64, 0>("random", []),
    ))
}

/// An index over a column of another table.
pub fn create_accounts_indexed_by_a_foreign_column() -> Plan {
    create_accounts().create_index(BTree::on_expr::<Account>(
        "accounts_foreign",
        Expr::<VarChar<120>>::call(
            "lower",
            [Expr::<VarChar<120>>::from_node(Author::NAME.into_node())],
        ),
    ))
}

/// Accounts with a unique index over the bucket divided by the float `2.0`:
/// buckets 4 and 5 stay distinct, which an integer `2` would not give on
/// SQLite and PostgreSQL.
pub fn create_accounts_with_float_expression() -> Plan {
    create_accounts().create_index(
        BTree::on_expr::<Account>(
            "accounts_half_bucket",
            Expr::<f64>::raw_op(
                "/",
                Expr::<i32>::from_node(Account::BUCKET.into_node()),
                bind(2.0_f64),
            ),
        )
        .unique(),
    )
}
