//! Index conditions and expressions built from another entity's columns are
//! refused, also when that entity has a column of the same name.

#[path = "../../rupa-core/tests/fixtures/mod.rs"]
mod fixtures;

use fixtures::User;
use rupa_core::dialect::{MySql, Postgres, Sqlite};
use rupa_core::ir::TableRef;
use rupa_core::prelude::*;
use rupa_core::schema::{BTree, Index, IndexColumn, IndexKind, SchemaOp, Table, Unique};
use rupa_core::{Column, DialectId, Scalar};
use rupa_sql::{ExprRefusal, RenderError, SchemaError, render_schema};

const ACCOUNTS: TableRef = TableRef::with_schema("billing", "accounts");

struct Account;

impl Account {
    const EMAIL: Column<Account, String, Scalar> = Column::scalar_in(ACCOUNTS, "email");
}

impl Relation for Account {
    const TABLE: TableRef = ACCOUNTS;
    const ID_COLUMNS: &'static [&'static str] = &[];
    const COLUMNS: &'static [rupa_core::ColumnMeta] = &[];
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

fn foreign(object: &'static str) -> RenderError {
    RenderError::Schema(Box::new(SchemaError::ForeignColumn {
        object,
        column: "email",
        recorded: ACCOUNTS,
        expected: User::TABLE,
    }))
}

#[test]
fn an_index_over_another_tables_handle_is_refused() {
    let index = Index::new(
        "users_email",
        User::TABLE,
        [IndexColumn::of(Account::EMAIL)],
    );

    assert_eq!(
        render_schema(&SchemaOp::CreateIndex(index), &Postgres),
        Err(foreign("users_email"))
    );
}

#[test]
fn a_unique_over_another_tables_handle_is_refused_in_both_forms() {
    let unique = Unique::on("users_email", Account::EMAIL).build();
    let create = SchemaOp::CreateTable(Table::of::<User>().unique(unique.clone()));
    let add = SchemaOp::AddConstraint {
        table: User::TABLE,
        constraint: unique.into(),
    };

    assert_eq!(
        render_schema(&create, &Postgres),
        Err(foreign("users_email"))
    );
    assert_eq!(render_schema(&add, &Postgres), Err(foreign("users_email")));
}

#[test]
fn index_and_unique_columns_of_the_same_table_or_of_none_are_accepted() {
    let own = Index::new("users_email", User::TABLE, [IndexColumn::of(User::EMAIL)]);
    let anonymous = Index::new(
        "users_email",
        User::TABLE,
        [IndexColumn::of(Anonymous::EMAIL)],
    );
    let unique = Unique::on("users_email", User::EMAIL).build();

    assert!(render_schema(&SchemaOp::CreateIndex(own), &Postgres).is_ok());
    assert!(render_schema(&SchemaOp::CreateIndex(anonymous), &Postgres).is_ok());
    assert!(
        render_schema(
            &SchemaOp::CreateTable(Table::of::<User>().unique(unique)),
            &Postgres
        )
        .is_ok()
    );
}
