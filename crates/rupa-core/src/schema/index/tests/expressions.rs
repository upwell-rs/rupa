use super::*;
use crate::column::{Column, Scalar};
use crate::expr::{Expr, ExprOps, IntoExpr};
use crate::ir::{BinOp, ColumnRef, ExprNode, TableRef};
use crate::schema::{DdlExpr, IndexExpr};

fn lower_title() -> Expr<String> {
    Expr::call("lower", [Expr::<String>::from_node(Doc::TITLE.into_node())])
}

fn json_expr() -> Expr<serde_json::Value> {
    Expr::from_node(ExprNode::Column(ColumnRef::new("meta")))
}

#[test]
fn expressions_become_index_parts_scoped_to_the_entity() {
    let index = BTree::on_expr::<Doc>("docs_title_lower", lower_title())
        .unique()
        .build();
    let [IndexPart::Expr(part)] = index.parts.as_slice() else {
        panic!("one expression part, got {:?}", index.parts);
    };

    assert_eq!(index.table, Doc::TABLE);
    assert!(index.unique);
    assert_eq!(part.ty, ColumnType::Text);
    assert_eq!(part.expr, DdlExpr::of::<Doc, String>(lower_title()));
    assert_eq!(part.expr.scope, Some(Doc::COLUMNS));
    assert_eq!(index.columns().count(), 0);
}

#[test]
fn columns_and_expressions_keep_their_order() {
    let index = BTree::on("docs_owner_title", Doc::OWNER)
        .and_expr(lower_title())
        .and(Doc::SLUG)
        .build();
    let shape: Vec<_> = index
        .parts
        .iter()
        .map(|part| matches!(part, IndexPart::Expr(_)))
        .collect();

    assert_eq!(shape, [false, true, false]);
    assert_eq!(index.columns().count(), 2);
}

#[test]
fn json_results_follow_the_json_kinds() {
    let gin = Gin::on_expr::<Doc>("docs_meta_expr", json_expr()).build();
    let btree = Index::on_expr::<Doc>("docs_meta_btree", json_expr());
    let nullable = Gin::on_expr::<Doc>(
        "docs_meta_nullable",
        Expr::<Option<serde_json::Value>>::from_node(ExprNode::Column(ColumnRef::new("meta"))),
    )
    .build();

    for index in [&gin, &btree, &nullable] {
        let [IndexPart::Expr(IndexExpr { ty, .. })] = index.parts.as_slice() else {
            panic!("one expression part, got {:?}", index.parts);
        };

        assert_eq!(*ty, ColumnType::Json);
    }
}

#[test]
fn a_filter_sets_the_predicate_scoped_to_the_entity() {
    let condition = Doc::OWNER.is_null();
    let index: Index = BTree::on("docs_unowned", Doc::SLUG)
        .filter(condition.clone())
        .into();

    assert_eq!(
        index.predicate,
        Some(Box::new(DdlExpr::of::<Doc, bool>(condition.clone())))
    );
    assert_eq!(
        index
            .predicate
            .as_ref()
            .and_then(|predicate| predicate.scope),
        Some(Doc::COLUMNS)
    );
    assert_eq!(Index::on("plain", Doc::SLUG).predicate, None);
    assert_eq!(
        Index::on("literal", Doc::SLUG)
            .filter(DdlExpr::new(condition.into_node()))
            .predicate
            .map(|predicate| predicate.scope),
        Some(None)
    );
}

#[test]
fn repeated_filters_are_anded_in_order() {
    let closed = Doc::OWNER.is_null();
    let titled = Doc::TITLE.eq("a");
    let index: Index = BTree::on("docs_unowned", Doc::SLUG)
        .filter(closed.clone())
        .filter(titled.clone())
        .into();
    let expected = ExprNode::Binary(
        BinOp::And,
        Box::new(closed.into_node()),
        Box::new(titled.clone().into_node()),
    );
    let literal = Index::on("literal", Doc::SLUG)
        .filter(DdlExpr::new(Doc::OWNER.is_null().into_node()))
        .filter(DdlExpr::of::<Doc, bool>(titled));
    let predicate = index.predicate.expect("a predicate");

    assert_eq!(predicate.node, expected);
    assert_eq!(predicate.scope, Some(Doc::COLUMNS));
    assert_eq!(
        literal.predicate.and_then(|predicate| predicate.scope),
        Some(Doc::COLUMNS)
    );
}

struct Other;

impl Other {
    const TITLE: Column<Other, String, Scalar> =
        Column::scalar_in(TableRef::new("others"), "title");
}

fn table_of(expr: &DdlExpr) -> Option<TableRef> {
    match &expr.node {
        ExprNode::Column(column) => column.table,
        ExprNode::Binary(_, lhs, _) => match &**lhs {
            ExprNode::Column(column) => column.table,
            other => panic!("not a column: {other:?}"),
        },
        other => panic!("not a column comparison: {other:?}"),
    }
}

#[test]
fn handles_record_their_entitys_table_in_conditions_and_expressions() {
    let filtered: Index = BTree::on("docs_slug", Doc::SLUG)
        .filter(Doc::TITLE.eq("a"))
        .into();
    let foreign: Index = BTree::on("docs_slug", Doc::SLUG)
        .filter(Other::TITLE.eq("a"))
        .into();
    let expression = BTree::on_expr::<Doc>(
        "docs_other_title",
        Expr::<String>::from_node(Other::TITLE.into_node()),
    )
    .build();
    let [IndexPart::Expr(part)] = expression.parts.as_slice() else {
        panic!("one expression part");
    };

    assert_eq!(
        table_of(filtered.predicate.as_ref().expect("a predicate")),
        Some(Doc::TABLE)
    );
    assert_eq!(
        table_of(foreign.predicate.as_ref().expect("a predicate")),
        Some(TableRef::new("others"))
    );
    assert_eq!(table_of(&part.expr), Some(TableRef::new("others")));
    assert_ne!(filtered.predicate, foreign.predicate);
}

#[test]
fn a_handle_without_a_table_leaves_its_node_untabled() {
    let bare: Column<Doc, String, Scalar> = Column::scalar("title");

    assert_eq!(bare.table(), None);
    assert_eq!(bare.column_ref(), ColumnRef::new("title"));
    assert_eq!(
        Doc::TITLE.column_ref(),
        ColumnRef::new("title").in_table(Doc::TABLE)
    );
    assert_eq!(
        Doc::TITLE.in_table(TableRef::new("x")).table(),
        Some(TableRef::new("x"))
    );
}
