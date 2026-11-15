use super::*;
use crate::column::{Column, Scalar};
use crate::entity::{Entity, FromRow};
use crate::error::ResultError;
use crate::ir::TableRef;
use crate::row::Row;
use crate::schema::{ColumnType, Table};
use crate::value::Value;
use crate::varchar::VarChar;

mod builders;
mod tables;

struct Doc;

impl Doc {
    const TITLE: Column<Doc, String, Scalar> = Column::scalar_in(<Doc as Entity>::TABLE, "title");
    const SLUG: Column<Doc, VarChar<64>, Scalar> =
        Column::scalar_in(<Doc as Entity>::TABLE, "slug");
    const OWNER: Column<Doc, i64, Scalar> = Column::scalar_in(<Doc as Entity>::TABLE, "owner");
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

fn docs() -> Table {
    Table::at(Doc::TABLE)
}
