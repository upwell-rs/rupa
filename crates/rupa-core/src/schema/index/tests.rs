use super::*;
use crate::column::Scalar;
use crate::dialect::DialectId;
use crate::entity::{Entity, FromRow};
use crate::error::ResultError;
use crate::row::Row;
use crate::value::Value;
use crate::varchar::VarChar;

struct Doc;

impl Doc {
    const TITLE: Column<Doc, String, Scalar> = Column::scalar("title");
    const SLUG: Column<Doc, VarChar<64>, Scalar> = Column::scalar("slug");
    const OWNER: Column<Doc, Option<i64>, Scalar> = Column::scalar("owner");
    const META: Column<Doc, serde_json::Value, crate::column::Json> = Column::json("meta");
    const TAGS: Column<Doc, Vec<String>, crate::column::Json> = Column::json("tags");
    const NOTE: Column<Doc, Option<Vec<String>>, crate::column::Json> =
        Column::nullable_json("note");
}

impl FromRow for Doc {
    fn from_row(_row: &dyn Row) -> Result<Self, ResultError> {
        Ok(Self)
    }
}

impl Entity for Doc {
    type Id = i64;
    const TABLE: TableRef = TableRef::new("docs");
    const ID_COLUMNS: &'static [&'static str] = &["id"];
    const COLUMNS: &'static [crate::column::ColumnMeta] = &[];

    fn id(&self) -> &i64 {
        &0
    }

    fn to_values(&self) -> Vec<Value> {
        Vec::new()
    }
}

struct Brin;

impl IndexKind for Brin {
    const METHOD: IndexMethod = IndexMethod::new("brin").postgres(IndexSupport::Using("brin"));
}

impl IndexableBy<Brin> for i64 {}

impl IndexableBy<Brin> for Option<i64> {}

#[test]
fn typed_handles_give_the_index_its_column_types() {
    let index = Index::on("docs_slug", Doc::SLUG).unique();

    assert_eq!(index.kind, BTree::METHOD);
    assert_eq!(index.table, Doc::TABLE);
    assert!(index.unique);
    assert_eq!(
        index.columns,
        [IndexColumn::new("slug", ColumnType::VarChar(64))]
    );
}

#[test]
fn builders_add_checked_columns_and_become_indexes() {
    let index: Index = BTree::on("docs_owner_title", Doc::OWNER)
        .and(Doc::TITLE)
        .unique()
        .into();

    assert_eq!(
        index.columns,
        [
            IndexColumn::new("owner", ColumnType::BigInt),
            IndexColumn::new("title", ColumnType::Text),
        ]
    );
    assert!(index.unique);
    assert_eq!(Gin::on("docs_meta", Doc::META).build().kind, Gin::METHOD);
}

#[test]
fn users_add_kinds_without_touching_the_crate() {
    let index = Brin::on("docs_owner", Doc::OWNER).build();

    assert_eq!(index.kind.name, "brin");
    assert_eq!(
        index.kind.support(DialectId::Postgres),
        IndexSupport::Using("brin")
    );
    assert!(matches!(
        index.kind.support(DialectId::MySql),
        IndexSupport::Unsupported(_)
    ));
}

#[test]
fn built_in_kinds_declare_their_dialects_and_limits() {
    assert_eq!(
        BTree::METHOD.support(DialectId::MySql),
        IndexSupport::Default
    );
    assert_eq!(
        Gist::METHOD.support(DialectId::Postgres),
        IndexSupport::Using("gist")
    );
    assert!(matches!(
        Gin::METHOD.support(DialectId::Memory),
        IndexSupport::Unsupported(_)
    ));
    let limits = |method: IndexMethod| (method.unique, method.multi_column);

    assert_eq!(limits(BTree::METHOD), (true, true));
    assert_eq!(limits(Hash::METHOD), (false, false));
}

#[test]
fn literal_columns_come_from_definitions() {
    let column = ColumnDef::typed::<VarChar<32>>("email");

    assert_eq!(
        IndexColumn::from(&column),
        IndexColumn::new("email", ColumnType::VarChar(32))
    );
}

#[test]
fn json_columns_of_any_type_are_accepted_by_json_kinds() {
    let index = Gin::on("docs_tags", Doc::TAGS)
        .and(Doc::META)
        .and(Doc::NOTE)
        .build();

    assert_eq!(index.kind, Gin::METHOD);
    assert_eq!(
        index.columns,
        [
            IndexColumn::new("tags", ColumnType::Json),
            IndexColumn::new("meta", ColumnType::Json),
            IndexColumn::new("note", ColumnType::Json),
        ]
    );
}
