use super::*;
use crate::expr::ExprOps;
use crate::schema::{DdlExpr, IndexColumn};

#[test]
fn a_typed_check_is_scoped_to_its_entity() {
    let check = Check::on::<Doc>("docs_owner_positive", Doc::OWNER.gt(0_i64));

    assert_eq!(check.name, "docs_owner_positive");
    assert_eq!(check.condition.scope, Some(Doc::COLUMNS));
    assert_eq!(
        check.condition,
        DdlExpr::of::<Doc, bool>(Doc::OWNER.gt(0_i64))
    );
}

#[test]
fn an_unchecked_check_has_no_scope() {
    let node = DdlExpr::of::<Doc, bool>(Doc::OWNER.gt(0_i64)).node;
    let check = Check::new("docs_literal", DdlExpr::new(node));

    assert_eq!(check.condition.scope, None);
}

#[test]
fn a_unique_builder_collects_typed_columns_in_order() {
    let unique = Unique::on("docs_owner_slug", Doc::OWNER)
        .and(Doc::SLUG)
        .build();

    assert_eq!(unique.name, "docs_owner_slug");
    assert_eq!(
        unique.columns,
        [
            IndexColumn::new("owner", ColumnType::BigInt),
            IndexColumn::new("slug", ColumnType::VarChar(64)),
        ]
    );
}

#[test]
fn a_literal_unique_takes_columns_directly() {
    let unique = Unique::new("docs_title", [IndexColumn::new("title", ColumnType::Text)]);

    assert_eq!(unique.columns.len(), 1);
}

#[test]
fn constraints_report_their_name_and_kind() {
    let check: Constraint = Check::on::<Doc>("c", Doc::OWNER.gt(0_i64)).into();
    let unique: Constraint = Unique::on("u", Doc::OWNER).and(Doc::TITLE).into();

    assert_eq!((check.name(), check.kind()), ("c", ConstraintKind::Check));
    assert_eq!(
        (unique.name(), unique.kind()),
        ("u", ConstraintKind::Unique)
    );
}
