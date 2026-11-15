//! A dynamic column names its table, and rendering refuses it anywhere else,
//! also when the table it belongs to has a column of the same name.

#[path = "../../rupa-core/tests/fixtures/mod.rs"]
mod fixtures;

use fixtures::User;
use rupa_core::dialect::Postgres;
use rupa_core::ir::TableRef;
use rupa_core::prelude::*;
use rupa_core::schema::{BTree, Check, Index, IndexKind, SchemaOp, Table, Unique};
use rupa_core::{Column, DialectId, Dynamic, Scalar};
use rupa_sql::{ExprRefusal, RenderError, SchemaError, render_schema};

const ACCOUNTS: TableRef = TableRef::with_schema("billing", "accounts");

fn account_email() -> Column<Dynamic, String, Scalar> {
    Column::dynamic::<String>(ACCOUNTS, "email")
}

fn user_email() -> Column<Dynamic, String, Scalar> {
    Column::dynamic::<String>(User::TABLE, "email")
}

fn refused(object: &'static str) -> Result<String, RenderError> {
    Err(RenderError::Schema(Box::new(SchemaError::Expression {
        object,
        dialect: DialectId::Postgres,
        reason: ExprRefusal::Column {
            column: "email",
            table: Some(ACCOUNTS),
            expected: User::TABLE,
        },
    })))
}

#[test]
fn an_index_filter_over_a_dynamic_column_of_another_table_is_refused() {
    let index = BTree::on("users_active", User::ACTIVE).filter(account_email().eq("a@b.c"));
    let op = SchemaOp::CreateIndex(index.into());

    assert_eq!(render_schema(&op, &Postgres), refused("users_active"));
}

#[test]
fn an_index_expression_over_a_dynamic_column_of_another_table_is_refused() {
    let lowered = Expr::<String>::call(
        "lower",
        [Expr::<String>::from_node(account_email().into_node())],
    );
    let index = BTree::on("users_active", User::ACTIVE).and_expr(lowered);
    let op = SchemaOp::CreateIndex(index.into());

    assert_eq!(render_schema(&op, &Postgres), refused("users_active"));
}

#[test]
fn a_check_over_a_dynamic_column_of_another_table_is_refused() {
    let check = Check::on::<User>("users_email_known", account_email().ne("x"));
    let op = SchemaOp::CreateTable(Table::of::<User>().check(check));

    assert_eq!(render_schema(&op, &Postgres), refused("users_email_known"));
}

#[test]
fn a_dynamic_column_of_the_same_table_renders() {
    let index = BTree::on("users_active", User::ACTIVE).filter(user_email().eq("a@b.c"));
    let rendered = render_schema(&SchemaOp::CreateIndex(index.into()), &Postgres);

    assert_eq!(
        rendered.as_deref(),
        Ok(r#"CREATE INDEX "users_active" ON "app"."users" ("active") WHERE "email" = 'a@b.c'"#)
    );
}

#[test]
fn a_dynamic_index_renders_on_the_table_its_handle_names() {
    let index = BTree::on_dynamic("accounts_email", account_email()).unique();
    let rendered = render_schema(&SchemaOp::CreateIndex(index.into()), &Postgres);

    assert_eq!(
        rendered.as_deref(),
        Ok(r#"CREATE UNIQUE INDEX "accounts_email" ON "billing"."accounts" ("email")"#)
    );
}

fn foreign(object: &'static str) -> RenderError {
    RenderError::Schema(Box::new(SchemaError::ForeignColumn {
        object,
        column: "email",
        recorded: ACCOUNTS,
        expected: User::TABLE,
    }))
}

#[test]
fn a_dynamic_column_of_another_table_added_to_an_index_is_refused() {
    let index: Index = BTree::on_dynamic("users_email", user_email())
        .and(account_email())
        .into();

    assert_eq!(
        render_schema(&SchemaOp::CreateIndex(index), &Postgres),
        Err(foreign("users_email"))
    );
}

#[test]
fn a_dynamic_column_of_another_table_in_a_unique_is_refused() {
    let unique = Unique::on_dynamic("users_email", account_email()).build();
    let op = SchemaOp::CreateTable(Table::of::<User>().unique(unique));

    assert_eq!(render_schema(&op, &Postgres), Err(foreign("users_email")));
}

#[test]
fn a_dynamic_unique_of_the_same_table_renders() {
    let unique = Unique::on_dynamic("users_email", user_email()).build();
    let op = SchemaOp::CreateTable(Table::of::<User>().unique(unique));

    assert!(render_schema(&op, &Postgres).is_ok());
}
