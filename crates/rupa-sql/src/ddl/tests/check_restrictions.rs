use rupa_core::ir::BinOp;
use rupa_core::schema::{Check, DdlExpr, ForeignKey};

use super::expressions::{binary, column, param, refused};
use super::*;
use crate::ExprRefusal;

fn check_on(name: &'static str, column_name: &'static str) -> Check {
    Check::new(
        name,
        DdlExpr::new(binary(BinOp::Gt, column(column_name), param(Value::I64(0)))),
    )
}

/// Notes whose `author_id` references authors with `action` on delete, and
/// a check over `author_id`.
fn with_author_key(action: ReferentialAction) -> Table {
    notes()
        .foreign_key(
            ForeignKey::new(
                ["author_id"],
                Reference {
                    table: TableRef::with_schema("app", "authors"),
                    columns: &["id"],
                },
            )
            .on_delete(action),
        )
        .check(check_on("notes_author_positive", "author_id"))
}

#[test]
fn mysql_refuses_a_check_over_the_auto_increment_column() {
    let table = notes().check(check_on("notes_id_positive", "id"));
    let results = all(&SchemaOp::CreateTable(table));

    assert_eq!(
        results[1],
        refused(
            "notes_id_positive",
            DialectId::MySql,
            ExprRefusal::AutoIncrementColumn { column: "id" }
        )
    );
    assert!(results[0].is_ok() && results[2].is_ok());
}

#[test]
fn mysql_refuses_a_check_over_a_cascading_foreign_key_column() {
    for action in [ReferentialAction::Cascade, ReferentialAction::SetNull] {
        let table = with_author_key(action);

        assert_eq!(
            render_schema(&SchemaOp::CreateTable(table), &MySql),
            refused(
                "notes_author_positive",
                DialectId::MySql,
                ExprRefusal::ReferentialActionColumn {
                    column: "author_id"
                }
            )
        );
    }
}

#[test]
fn mysql_allows_a_check_over_a_restricting_foreign_key_column() {
    for action in [ReferentialAction::NoAction, ReferentialAction::Restrict] {
        let table = with_author_key(action);

        assert!(render_schema(&SchemaOp::CreateTable(table), &MySql).is_ok());
    }
}

#[test]
fn a_restricted_column_is_not_refused_when_added_to_an_existing_table() {
    let op = SchemaOp::AddConstraint {
        table: NOTES,
        constraint: check_on("notes_id_positive", "id").into(),
    };

    assert!(render_schema(&op, &MySql).is_ok());
}
