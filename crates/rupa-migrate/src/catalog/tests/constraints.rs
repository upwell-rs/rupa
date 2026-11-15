use rupa_core::Value;
use rupa_core::ir::{BinOp, ExprNode, TableRef};
use rupa_core::schema::{
    Check, ColumnType, Constraint, ConstraintKind, DdlExpr, IndexColumn, Unique,
};

use super::expressions::column;
use super::plans::{NOTES, assert_distinct_digests, notes, plan_digest};
use super::*;

fn longer_than(column_name: &'static str, n: i64) -> DdlExpr {
    DdlExpr::new(ExprNode::Binary(
        BinOp::Gt,
        Box::new(column(column_name)),
        Box::new(ExprNode::Param(Value::I64(n))),
    ))
}

fn check() -> Check {
    Check::new("notes_author_positive", longer_than("author_id", 0))
}

fn unique() -> Unique {
    Unique::new(
        "notes_title_author",
        [
            IndexColumn::new("title", ColumnType::Text),
            IndexColumn::new("author_id", ColumnType::BigInt),
        ],
    )
}

/// Pins the encodings of constraints, which `schema_encoding_is_stable`
/// cannot reach. Changing this value changes applied checksums: it needs a
/// new `CHECKSUM_FORMAT`, never an updated expectation.
#[test]
fn constraint_encoding_is_stable() {
    let checksum = plan_digest(|| {
        Plan::new()
            .create_table(notes().check(check()).unique(unique()))
            .add_constraint(NOTES, check())
            .add_constraint(NOTES, unique())
            .drop_constraint(NOTES, "notes_author_positive", ConstraintKind::Check)
            .drop_constraint(NOTES, "notes_title_author", ConstraintKind::Unique)
    });

    assert_eq!(
        checksum.to_string(),
        "65d625339fd5cb6422d7961dab55785cf697a568506573e02be837a5a70d88ad"
    );
}

#[test]
fn every_field_of_a_constraint_is_checksummed() {
    let variants: [fn() -> Plan; 14] = [
        || Plan::new().create_table(notes()),
        || Plan::new().create_table(notes().check(check())),
        || Plan::new().create_table(notes().unique(unique())),
        || Plan::new().create_table(notes().check(check()).unique(unique())),
        || Plan::new().create_table(notes().unique(unique()).check(check())),
        || {
            Plan::new()
                .create_table(notes().check(Check::new("other", longer_than("author_id", 0))))
        },
        || {
            Plan::new().create_table(notes().check(Check::new(
                "notes_author_positive",
                longer_than("author_id", 1),
            )))
        },
        || {
            Plan::new().create_table(
                notes().check(Check::new("notes_author_positive", longer_than("title", 0))),
            )
        },
        || Plan::new().create_table(notes().unique(Unique::new("other", unique().columns))),
        || {
            Plan::new().create_table(notes().unique(Unique::new(
                "notes_title_author",
                [IndexColumn::new("title", ColumnType::Text)],
            )))
        },
        || Plan::new().add_constraint(NOTES, check()),
        || Plan::new().add_constraint(NOTES, unique()),
        || Plan::new().add_constraint(TableRef::new("other"), check()),
        || Plan::new().drop_constraint(NOTES, "notes_author_positive", ConstraintKind::Check),
    ];

    assert_distinct_digests(&variants, "every constraint field changes the checksum");
}

#[test]
fn a_dropped_constraints_kind_and_name_are_checksummed() {
    let variants: [fn() -> Plan; 4] = [
        || Plan::new().drop_constraint(NOTES, "c", ConstraintKind::Check),
        || Plan::new().drop_constraint(NOTES, "c", ConstraintKind::Unique),
        || Plan::new().drop_constraint(NOTES, "d", ConstraintKind::Check),
        || Plan::new().drop_constraint(TableRef::new("other"), "c", ConstraintKind::Check),
    ];

    assert_distinct_digests(&variants, "a drop is identified by table, name and kind");
}

#[test]
fn a_uniques_column_types_are_not_part_of_the_checksum() {
    let retyped = || {
        Plan::new().add_constraint(
            NOTES,
            Unique::new(
                "notes_title_author",
                [
                    IndexColumn::new("title", ColumnType::VarChar(10)),
                    IndexColumn::new("author_id", ColumnType::Int),
                ],
            ),
        )
    };

    assert_eq!(
        plan_digest(retyped),
        plan_digest(|| Plan::new().add_constraint(NOTES, Constraint::from(unique())))
    );
}
