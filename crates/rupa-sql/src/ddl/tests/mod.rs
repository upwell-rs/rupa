use rupa_core::schema::{
    BTree, ColumnDefault, ColumnType, Gin, Hash, Index, IndexColumn, IndexKind, ReferentialAction,
};
use rupa_core::{MySql, Postgres, Reference, Sqlite, Value};

use super::*;

mod alterations;
mod indexes;
mod kinds;
mod tables;

const NOTES: TableRef = TableRef::with_schema("app", "notes");

/// `op` rendered for PostgreSQL, MySQL and SQLite.
fn all(op: &SchemaOp) -> [Result<String, RenderError>; 3] {
    [
        render_schema(op, &Postgres),
        render_schema(op, &MySql),
        render_schema(op, &Sqlite),
    ]
}

fn ok(op: &SchemaOp) -> [String; 3] {
    all(op).map(|sql| sql.expect("renderable on every dialect"))
}

fn notes() -> Table {
    Table::at(NOTES)
        .column(ColumnDef::new("id", ColumnType::BigInt).identity())
        .column(ColumnDef::new("title", ColumnType::VarChar(200)).unique())
        .column(ColumnDef::new("body", ColumnType::Text).nullable())
        .column(
            ColumnDef::new("pinned", ColumnType::Bool)
                .default(ColumnDefault::Value(Value::Bool(false))),
        )
        .column(ColumnDef::new("author_id", ColumnType::BigInt).nullable())
        .primary_key(["id"])
        .foreign_key(
            ForeignKey::new(
                ["author_id"],
                Reference {
                    table: TableRef::with_schema("app", "authors"),
                    columns: &["id"],
                },
            )
            .on_delete(ReferentialAction::SetNull),
        )
}

fn index<const N: usize>(
    name: &'static str,
    kind: rupa_core::schema::IndexMethod,
    columns: [(&'static str, ColumnType); N],
) -> Index {
    Index::new(
        name,
        NOTES,
        columns.map(|(name, ty)| IndexColumn::new(name, ty)),
    )
    .method(kind)
}

fn refusal(index: &'static str, dialect: DialectId, reason: IndexRefusal) -> RenderError {
    RenderError::Schema(Box::new(SchemaError::Index {
        index,
        dialect,
        reason,
    }))
}
