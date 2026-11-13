//! Index conditions and expressions built from another entity's columns are
//! refused, also when that entity has a column of the same name.

#[path = "../../rupa-core/tests/fixtures/mod.rs"]
mod fixtures;

use fixtures::User;
use rupa_core::dialect::{MySql, Postgres, Sqlite};
use rupa_core::ir::TableRef;
use rupa_core::prelude::*;
use rupa_core::schema::{BTree, IndexKind, SchemaOp};
use rupa_core::{Column, DialectId, Scalar};
use rupa_sql::{ExprRefusal, RenderError, SchemaError, render_schema};

const ACCOUNTS: TableRef = TableRef::with_schema("billing", "accounts");

struct Account;

impl Account {
    const EMAIL: Column<Account, String, Scalar> = Column::scalar_in(ACCOUNTS, "email");
}

struct Anonymous;

impl Anonymous {
    const EMAIL: Column<Anonymous, String, Scalar> = Column::scalar("email");
}

fn refused(index: &'static str, dialect: DialectId) -> RenderError {
    RenderError::Schema(Box::new(SchemaError::Expression {
        object: index,
        dialect,
        reason: ExprRefusal::Column {
            column: "email",
            table: Some(ACCOUNTS),
            expected: User::TABLE,
        },
    }))
}

#[test]
fn a_filter_over_another_tables_same_named_column_is_refused() {
    let index = BTree::on("users_active", User::ACTIVE).filter(Account::EMAIL.eq("a@b.c"));
    let op = SchemaOp::CreateIndex(index.into());

    assert_eq!(
        render_schema(&op, &Postgres),
        Err(refused("users_active", DialectId::Postgres))
    );
    assert_eq!(
        render_schema(&op, &Sqlite),
        Err(refused("users_active", DialectId::Sqlite))
    );
}

#[test]
fn an_expression_over_another_tables_same_named_column_is_refused() {
    let foreign = Expr::<String>::call(
        "lower",
        [Expr::<String>::from_node(Account::EMAIL.into_node())],
    );
    let index = BTree::on("users_active", User::ACTIVE).and_expr(foreign);

    assert_eq!(
        render_schema(&SchemaOp::CreateIndex(index.into()), &Postgres),
        Err(refused("users_active", DialectId::Postgres))
    );
}

#[test]
fn the_message_names_both_tables() {
    let index = BTree::on("users_active", User::ACTIVE).filter(Account::EMAIL.eq("x"));
    let error = render_schema(&SchemaOp::CreateIndex(index.into()), &Postgres)
        .expect_err("refused")
        .to_string();

    assert!(error.contains("`email`"), "{error}");
    assert!(error.contains("billing.accounts"), "{error}");
    assert!(error.contains("`app.users`"), "{error}");
}

#[test]
fn the_entitys_own_columns_render() {
    let index = BTree::on("users_active", User::ACTIVE).filter(User::EMAIL.eq("a@b.c"));
    let rendered = render_schema(&SchemaOp::CreateIndex(index.into()), &Postgres);

    assert_eq!(
        rendered.as_deref(),
        Ok(r#"CREATE INDEX "users_active" ON "app"."users" ("active") WHERE "email" = 'a@b.c'"#)
    );
}

#[test]
fn a_handle_without_a_table_is_checked_by_name_only() {
    let index = BTree::on("users_active", User::ACTIVE).filter(Anonymous::EMAIL.eq("a@b.c"));

    assert!(render_schema(&SchemaOp::CreateIndex(index.into()), &Postgres).is_ok());
}

#[test]
fn mysql_still_refuses_the_partial_index_first() {
    let index = BTree::on("users_active", User::ACTIVE).filter(Account::EMAIL.eq("x"));

    assert!(matches!(
        render_schema(&SchemaOp::CreateIndex(index.into()), &MySql),
        Err(RenderError::Schema(schema)) if matches!(*schema, SchemaError::Index { .. })
    ));
}
