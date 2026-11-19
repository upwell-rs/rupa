use super::*;
use crate::expr::ExprOps;
use crate::ir::ExprNode;

const LEDGER: TableRef = TableRef::with_schema("billing", "ledger");
const OTHER: TableRef = TableRef::new("other");

#[test]
fn a_dynamic_unique_records_the_table_of_each_handle() {
    let unique = Unique::on_dynamic(
        "ledger_owner_age",
        Column::dynamic::<String>(LEDGER, "owner"),
    )
    .and(Column::dynamic::<i64>(LEDGER, "age"))
    .build();

    assert_eq!(
        unique.columns,
        [
            IndexColumn::new("owner", ColumnType::Text).in_table(LEDGER),
            IndexColumn::new("age", ColumnType::BigInt).in_table(LEDGER),
        ]
    );
}

#[test]
fn a_dynamic_unique_keeps_a_column_of_another_table_for_rendering_to_refuse() {
    let unique = Unique::on_dynamic("ledger_age", Column::dynamic::<i64>(LEDGER, "age"))
        .and(Column::dynamic::<i64>(OTHER, "age"))
        .build();

    assert_eq!(
        unique.columns.iter().map(|c| c.table).collect::<Vec<_>>(),
        [Some(LEDGER), Some(OTHER)]
    );
}

#[test]
fn a_check_over_a_dynamic_handle_keeps_the_handles_table_in_its_node() {
    let age = Column::dynamic::<i64>(LEDGER, "age");
    let check = Check::on::<Doc>("ledger_age_positive", age.gt(0_i64));
    let ExprNode::Binary(_, lhs, _) = &check.condition.node else {
        panic!("a comparison, got {:?}", check.condition.node);
    };

    assert_eq!(**lhs, ExprNode::Column(age.column_ref()));
    assert_eq!(age.column_ref().table, Some(LEDGER));
}
