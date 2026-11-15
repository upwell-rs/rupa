use super::*;
use crate::column::Dynamic;
use crate::ir::ExprNode;
use crate::schema::ColumnType;

const LEDGER: TableRef = TableRef::with_schema("billing", "ledger");
const OTHER: TableRef = TableRef::new("other");

fn ledger_column(name: &'static str, ty: ColumnType) -> IndexColumn {
    IndexColumn {
        table: Some(LEDGER),
        ..IndexColumn::new(name, ty)
    }
}

#[test]
fn a_dynamic_handle_indexes_the_table_it_names() {
    let age = Column::dynamic::<i64>(LEDGER, "age");
    let index = BTree::on_dynamic("ledger_age", age).unique().build();

    assert_eq!(index.table, LEDGER);
    assert!(index.unique);
    assert_eq!(
        index.columns().copied().collect::<Vec<_>>(),
        [ledger_column("age", ColumnType::BigInt)]
    );
}

#[test]
fn a_dynamic_index_is_the_literal_one_with_recorded_tables() {
    let dynamic: Index = BTree::on_dynamic("ledger_age", Column::dynamic::<i64>(LEDGER, "age"))
        .and(Column::dynamic::<String>(LEDGER, "owner"))
        .into();
    let literal = Index::new(
        "ledger_age",
        LEDGER,
        [
            IndexColumn {
                table: Some(LEDGER),
                ..IndexColumn::new("age", ColumnType::BigInt)
            },
            IndexColumn {
                table: Some(LEDGER),
                ..IndexColumn::new("owner", ColumnType::Text)
            },
        ],
    );

    assert_eq!(dynamic, literal);
}

#[test]
fn dynamic_handles_check_the_kind_like_typed_ones() {
    let tags = Column::dynamic_json::<Vec<String>>(LEDGER, "tags");
    let index = Gin::on_dynamic("ledger_tags", tags).build();

    assert_eq!(index.kind, Gin::METHOD);
}

#[test]
fn a_dynamic_filter_keeps_the_handles_table_in_its_condition() {
    let active = Column::dynamic::<bool>(LEDGER, "active");
    let index = BTree::on_dynamic("ledger_age", Column::dynamic::<i64>(LEDGER, "age"))
        .filter(active)
        .build();

    assert_eq!(index.predicates.len(), 1);
    assert_eq!(index.predicates[0].scope, None);
    assert_eq!(
        index.predicates[0].node,
        ExprNode::Column(active.column_ref())
    );
    assert_eq!(active.column_ref().table, Some(LEDGER));
}

#[test]
fn a_dynamic_handle_is_typed_by_its_value() {
    let _: Column<Dynamic, i64, Scalar> = Column::dynamic::<i64>(LEDGER, "age");
    let _: Column<Dynamic, Option<Vec<String>>, crate::column::Json> =
        Column::dynamic_nullable_json::<Vec<String>>(LEDGER, "note");

    assert_eq!(Column::dynamic::<i64>(LEDGER, "age").table(), Some(LEDGER));
}

#[test]
fn a_column_of_another_table_is_kept_for_rendering_to_refuse() {
    let index = BTree::on_dynamic("ledger_age", Column::dynamic::<i64>(LEDGER, "age"))
        .and(Column::dynamic::<i64>(OTHER, "age"))
        .build();

    assert_eq!(
        index.columns().map(|c| c.table).collect::<Vec<_>>(),
        [Some(LEDGER), Some(OTHER)]
    );
}
