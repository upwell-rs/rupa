use rupa_core::Column;
use rupa_core::entity::Entity;
use rupa_core::ir::TableRef;
use rupa_core::schema::{ColumnDef, Index, SchemaOp, Table};

use crate::Operation;

/// The ordered operations of a planned migration: SQL scripts and portable
/// schema changes, in any mix.
///
/// ```ignore
/// Plan::new()
///     .create_table(Table::of::<Tag>())
///     .add_column(Note::SLUG)
///     .create_index(Index::on("notes_slug", Note::SLUG).unique())
///     .sql("UPDATE notes SET slug = lower(title)")
/// ```
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Plan {
    operations: Vec<Operation>,
}

impl Plan {
    /// An empty plan.
    pub const fn new() -> Self {
        Self {
            operations: Vec::new(),
        }
    }

    /// The operations, in execution order.
    pub fn operations(&self) -> &[Operation] {
        &self.operations
    }

    /// Appends `operation`.
    pub fn then(mut self, operation: Operation) -> Self {
        self.operations.push(operation);

        self
    }

    /// Appends a dialect-native script.
    pub fn sql(self, sql: &'static str) -> Self {
        self.then(Operation::Sql(sql))
    }

    /// Appends a schema change.
    pub fn schema(self, op: SchemaOp) -> Self {
        self.then(Operation::Schema(op))
    }

    /// `CREATE TABLE`.
    pub fn create_table(self, table: Table) -> Self {
        self.schema(SchemaOp::CreateTable(table))
    }

    /// `DROP TABLE`.
    pub fn drop_table(self, table: TableRef) -> Self {
        self.schema(SchemaOp::DropTable(table))
    }

    /// Renames `table` to `to`, within its schema.
    pub fn rename_table(self, table: TableRef, to: &'static str) -> Self {
        self.schema(SchemaOp::RenameTable { table, to })
    }

    /// Adds the column an entity field handle describes to its entity's table.
    pub fn add_column<E: Entity, T, K>(self, column: Column<E, T, K>) -> Self {
        self.add_column_to(E::TABLE, ColumnDef::of(column))
    }

    /// Adds `column` to `table`.
    pub fn add_column_to(self, table: TableRef, column: ColumnDef) -> Self {
        self.schema(SchemaOp::AddColumn { table, column })
    }

    /// Drops `column` from `table`.
    pub fn drop_column(self, table: TableRef, column: &'static str) -> Self {
        self.schema(SchemaOp::DropColumn { table, column })
    }

    /// Renames column `from` of `table` to `to`.
    pub fn rename_column(self, table: TableRef, from: &'static str, to: &'static str) -> Self {
        self.schema(SchemaOp::RenameColumn { table, from, to })
    }

    /// `CREATE [UNIQUE] INDEX`.
    pub fn create_index(self, index: Index) -> Self {
        self.schema(SchemaOp::CreateIndex(index))
    }

    /// Drops index `name` of `table`.
    pub fn drop_index(self, table: TableRef, name: &'static str) -> Self {
        self.schema(SchemaOp::DropIndex { table, name })
    }
}
