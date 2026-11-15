use super::*;
use crate::column::{Column, Scalar};
use crate::entity::Relation;
use crate::ir::TableRef;
use crate::schema::{ColumnType, Table};
use crate::varchar::VarChar;

mod builders;
mod dynamic;
mod tables;

struct Doc;

impl Doc {
    const TITLE: Column<Doc, String, Scalar> = Column::scalar_in(<Doc as Relation>::TABLE, "title");
    const SLUG: Column<Doc, VarChar<64>, Scalar> =
        Column::scalar_in(<Doc as Relation>::TABLE, "slug");
    const OWNER: Column<Doc, i64, Scalar> = Column::scalar_in(<Doc as Relation>::TABLE, "owner");
}

impl Relation for Doc {
    const TABLE: TableRef = TableRef::new("docs");
    const ID_COLUMNS: &'static [&'static str] = &["id"];
    const COLUMNS: &'static [crate::column::ColumnMeta] = &[];
}

fn docs() -> Table {
    Table::at(Doc::TABLE)
}
