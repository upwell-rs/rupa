use rupa_core::ir::{BinOp, ExprNode, UnOp};
use rupa_core::schema::DdlExpr;

use super::expressions::{SCOPE, binary, column, param, partial, refused};
use super::*;
use crate::ExprRefusal;

fn scoped(node: ExprNode) -> DdlExpr {
    DdlExpr {
        node,
        scope: Some(SCOPE),
    }
}

fn legacy_is_null() -> ExprNode {
    ExprNode::Unary(UnOp::IsNull, Box::new(column("legacy_flag")))
}

fn small() -> ExprNode {
    binary(BinOp::Lt, column("n"), param(Value::I32(10)))
}

#[test]
fn repeated_filters_are_parenthesized_and_anded_in_order() {
    let index = partial(legacy_is_null()).filter(DdlExpr::new(small()));

    assert_eq!(
        render_schema(&SchemaOp::CreateIndex(index), &Sqlite).as_deref(),
        Ok(
            r#"CREATE INDEX "app"."notes_partial" ON "notes" ("title") WHERE ("legacy_flag" IS NULL) AND ("n" < 10)"#
        )
    );
}

#[test]
fn an_unchecked_filter_stays_unchecked_beside_a_checked_one() {
    let index = partial(legacy_is_null()).filter(scoped(small()));

    assert!(
        render_schema(&SchemaOp::CreateIndex(index), &Postgres).is_ok(),
        "legacy_flag is outside SCOPE but its filter is unchecked"
    );
}

#[test]
fn a_checked_filter_is_checked_against_its_own_scope() {
    let index = partial(legacy_is_null()).filter(scoped(binary(
        BinOp::Eq,
        column("price"),
        param(Value::I32(1)),
    )));

    assert_eq!(
        render_schema(&SchemaOp::CreateIndex(index), &Postgres),
        refused(
            "notes_partial",
            DialectId::Postgres,
            ExprRefusal::Column {
                column: "price",
                table: None,
                expected: NOTES
            }
        )
    );
}
