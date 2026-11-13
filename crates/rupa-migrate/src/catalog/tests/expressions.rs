use rupa_core::SqlType;
use rupa_core::Value;
use rupa_core::ir::TableRef;
use rupa_core::ir::{BinOp, ColumnRef, ExprNode, PathSeg, UnOp};
use rupa_core::schema::{ColumnType, DdlExpr, Index, IndexColumn, IndexExpr, IndexPart};

use super::plans::{NOTES, assert_distinct_digests, notes_title, plan_digest};
use super::*;

pub(super) fn column(name: &'static str) -> ExprNode {
    ExprNode::Column(ColumnRef::new(name))
}

pub(super) fn call(name: &'static str, node: ExprNode) -> ExprNode {
    ExprNode::Call(name, vec![node])
}

fn lower_title() -> ExprNode {
    call("lower", column("title"))
}

fn closed() -> ExprNode {
    ExprNode::Unary(UnOp::IsNull, Box::new(column("closed_at")))
}

fn expression(node: ExprNode) -> IndexPart {
    IndexExpr {
        expr: DdlExpr::new(node),
        ty: ColumnType::Text,
    }
    .into()
}

pub(super) fn on(node: ExprNode) -> Index {
    Index {
        parts: vec![expression(node)],
        ..notes_title()
    }
}

pub(super) fn filtered(node: ExprNode) -> Index {
    notes_title().filter(DdlExpr::new(node))
}

fn equals(value: Value) -> ExprNode {
    ExprNode::Binary(
        BinOp::Eq,
        Box::new(column("title")),
        Box::new(ExprNode::Param(value)),
    )
}

#[test]
fn expressions_and_predicates_are_checksummed() {
    let variants: [fn() -> Plan; 22] = [
        || Plan::new().create_index(notes_title()),
        || Plan::new().create_index(on(lower_title())),
        || Plan::new().create_index(on(call("upper", column("title")))),
        || Plan::new().create_index(on(call("lower", column("body")))),
        || Plan::new().create_index(on(lower_title()).unique()),
        || Plan::new().create_index(on(column("title"))),
        || {
            let mut index = on(lower_title());

            index.parts.push(notes_title().parts.remove(0));

            Plan::new().create_index(index)
        },
        || {
            let mut index = on(lower_title());

            index.parts.insert(0, notes_title().parts.remove(0));

            Plan::new().create_index(index)
        },
        || Plan::new().create_index(filtered(closed())),
        || {
            Plan::new().create_index(filtered(ExprNode::Unary(
                UnOp::IsNotNull,
                Box::new(column("closed_at")),
            )))
        },
        || {
            Plan::new().create_index(filtered(ExprNode::Unary(
                UnOp::IsNull,
                Box::new(column("opened_at")),
            )))
        },
        || Plan::new().create_index(on(lower_title()).filter(DdlExpr::new(closed()))),
        || Plan::new().create_index(filtered(equals(Value::Text("open".into())))),
        || Plan::new().create_index(filtered(equals(Value::Text("shut".into())))),
        || Plan::new().create_index(filtered(equals(Value::I64(1)))),
        || Plan::new().create_index(filtered(equals(Value::I32(1)))),
        || Plan::new().create_index(filtered(equals(Value::Null(SqlType::Text)))),
        || Plan::new().create_index(filtered(equals(Value::Null(SqlType::I64)))),
        || {
            Plan::new().create_index(filtered(ExprNode::Cast(
                Box::new(column("title")),
                SqlType::I64,
            )))
        },
        || {
            Plan::new().create_index(filtered(ExprNode::JsonPath {
                column: ColumnRef::new("meta"),
                path: vec![PathSeg::Key("a")],
                as_text: true,
            }))
        },
        || {
            Plan::new().create_index(filtered(ExprNode::JsonPath {
                column: ColumnRef::new("meta"),
                path: vec![PathSeg::Index(0)],
                as_text: true,
            }))
        },
        || {
            Plan::new().create_index(filtered(ExprNode::RawOp(
                "ILIKE",
                Box::new(column("title")),
                Box::new(ExprNode::Param(Value::Text("a".into()))),
            )))
        },
    ];

    assert_distinct_digests(&variants, "every expression detail is part of the identity");
}

#[test]
fn a_columns_name_cannot_pass_for_an_expression() {
    let variants: [fn() -> Plan; 3] = [
        || {
            Plan::new().create_index(Index {
                parts: vec![IndexColumn::new("expression-parts", ColumnType::Text).into()],
                ..notes_title()
            })
        },
        || Plan::new().create_index(on(column("expression-parts"))),
        || Plan::new().create_index(filtered(column("predicate"))),
    ];

    assert_distinct_digests(&variants, "tags and names are told apart by position");
}

#[test]
fn an_expressions_result_type_and_scope_are_not_identity() {
    let retyped = || {
        let mut index = on(lower_title());

        index.parts = vec![
            IndexExpr {
                expr: DdlExpr::new(lower_title()),
                ty: ColumnType::VarChar(10),
            }
            .into(),
        ];

        Plan::new().create_index(index)
    };

    assert_eq!(
        plan_digest(|| Plan::new().create_index(on(lower_title()))),
        plan_digest(retyped)
    );
}

/// Pins the encodings of an expression index and of a partial index, which
/// `schema_encoding_is_stable` cannot reach. Changing this value changes
/// applied checksums: it needs a new `CHECKSUM_FORMAT`, never an updated
/// expectation.
#[test]
fn expression_and_predicate_encoding_is_stable() {
    let checksum = plan_digest(|| {
        Plan::new()
            .create_index(on(lower_title()).unique())
            .create_index(filtered(closed()))
    });

    assert_eq!(
        checksum.to_string(),
        "195764919efb93450f134a2c1f8a183c077cac33768a4df5ea495aaefc81b2fb"
    );
}

fn titled(table: Option<TableRef>) -> Plan {
    let column = match table {
        Some(table) => ColumnRef::new("title").in_table(table),
        None => ColumnRef::new("title"),
    };

    Plan::new().create_index(on(call("lower", ExprNode::Column(column))))
}

#[test]
fn a_columns_recorded_table_is_not_part_of_the_checksum() {
    let untabled = plan_digest(|| titled(None));
    let notes = plan_digest(|| titled(Some(NOTES)));
    let elsewhere = plan_digest(|| titled(Some(TableRef::new("elsewhere"))));

    assert_eq!(untabled, notes);
    assert_eq!(notes, elsewhere);
}
