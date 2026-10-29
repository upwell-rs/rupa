//! Dialect-free schema operations (DDL).
//!
//! Operations are plain data: a migration plan holding them can be inspected,
//! checksummed and rendered for any dialect without a connection. Rendering
//! lives in `rupa-sql`, which rejects what a dialect cannot express instead
//! of approximating it.
//!
//! Like the query IR, every identifier and SQL fragment is `&'static str`:
//! author-controlled source text, never runtime input. Default values are the
//! only data, and renderers write them as checked literals.
//!
//! Definitions can be written from literals ([`Table::named`],
//! [`ColumnDef::new`]) or derived from entities ([`Table::of`],
//! [`ColumnDef::of`]), and adjusted afterwards.

mod build;

use crate::column::{Generated, Reference};
use crate::ir::TableRef;
use crate::value::{SqlType, Value};

/// One schema change. Deliberately exhaustive: a new operation must break
/// every renderer's and every checksum encoder's match.
#[derive(Debug, Clone, PartialEq)]
pub enum SchemaOp {
    /// `CREATE TABLE`.
    CreateTable(Table),
    /// `DROP TABLE`.
    DropTable(TableRef),
    /// Renames a table within its schema.
    RenameTable {
        /// The table to rename.
        table: TableRef,
        /// The new name.
        to: &'static str,
    },
    /// Adds a column to an existing table.
    AddColumn {
        /// The table to change.
        table: TableRef,
        /// The new column.
        column: ColumnDef,
    },
    /// Drops a column.
    DropColumn {
        /// The table to change.
        table: TableRef,
        /// The column to drop.
        column: &'static str,
    },
    /// Renames a column.
    RenameColumn {
        /// The table to change.
        table: TableRef,
        /// The current column name.
        from: &'static str,
        /// The new column name.
        to: &'static str,
    },
    /// `CREATE [UNIQUE] INDEX`.
    CreateIndex(Index),
    /// `DROP INDEX`. The table is needed by dialects whose indexes belong to
    /// a table (MySQL) and gives the schema elsewhere.
    DropIndex {
        /// The indexed table.
        table: TableRef,
        /// The index name.
        name: &'static str,
    },
}

/// A portable column type. Each dialect renders it as the type its driver
/// binds and decodes values of the matching [`SqlType`] with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColumnType {
    /// A boolean.
    Bool,
    /// A 16-bit integer.
    SmallInt,
    /// A 32-bit integer.
    Int,
    /// A 64-bit integer.
    BigInt,
    /// A 32-bit float.
    Real,
    /// A 64-bit float.
    Double,
    /// An exact decimal with `precision` digits, `scale` after the point.
    Decimal {
        /// Total digits.
        precision: u8,
        /// Digits after the decimal point.
        scale: u8,
    },
    /// Unbounded text.
    Text,
    /// Text of at most `n` characters; indexable on every dialect.
    VarChar(u32),
    /// Unbounded bytes.
    Bytes,
    /// A UUID.
    Uuid,
    /// A calendar date.
    Date,
    /// A time of day.
    Time,
    /// A date and time without a time zone.
    Timestamp,
    /// An instant, stored in UTC.
    TimestampTz,
    /// A JSON document.
    Json,
    /// A dialect-specific type, written verbatim where given.
    Custom(CustomType),
}

impl From<SqlType> for ColumnType {
    fn from(ty: SqlType) -> Self {
        Self::from_sql(ty)
    }
}

/// Per-dialect type names for [`ColumnType::Custom`]. A dialect without one
/// rejects the operation rather than guessing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct CustomType {
    /// The PostgreSQL type.
    pub postgres: Option<&'static str>,
    /// The MySQL type.
    pub mysql: Option<&'static str>,
    /// The SQLite type.
    pub sqlite: Option<&'static str>,
}

/// A column's default.
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnDefault {
    /// A literal value (booleans, numbers and text).
    Value(Value),
    /// A static SQL expression, written verbatim (`CURRENT_TIMESTAMP`).
    Sql(&'static str),
}

/// One column of a table.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnDef {
    /// The column name.
    pub name: &'static str,
    /// The column type.
    pub ty: ColumnType,
    /// Whether the column admits `NULL`.
    pub nullable: bool,
    /// How the database supplies the value, if it does. A database-generated
    /// column needs a [`default`](Self::default); an identity needs an
    /// integer type.
    pub generated: Option<Generated>,
    /// The default value.
    pub default: Option<ColumnDefault>,
    /// A single-column `UNIQUE` constraint.
    pub unique: bool,
}

/// `CREATE TABLE`: columns, primary key and foreign keys.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    /// The table, with its schema if any.
    pub table: TableRef,
    /// The columns, in order.
    pub columns: Vec<ColumnDef>,
    /// Primary key column names, in key order.
    pub primary_key: Vec<&'static str>,
    /// Foreign keys.
    pub foreign_keys: Vec<ForeignKey>,
}

/// A foreign key constraint.
#[derive(Debug, Clone, PartialEq)]
pub struct ForeignKey {
    /// The referencing columns, in the referenced columns' order.
    pub columns: Vec<&'static str>,
    /// The referenced table and columns.
    pub references: Reference,
    /// What deleting a referenced row does.
    pub on_delete: ReferentialAction,
}

/// What deleting a referenced row does to referencing rows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum ReferentialAction {
    /// Fail at the end of the statement (every dialect's default).
    #[default]
    NoAction,
    /// Fail immediately.
    Restrict,
    /// Delete the referencing rows.
    Cascade,
    /// Set the referencing columns to `NULL`.
    SetNull,
}

/// `CREATE [UNIQUE] INDEX`. It names columns without their types: on MySQL,
/// index `VarChar` columns rather than unbounded `Text`, which the server
/// cannot index without a prefix length.
#[derive(Debug, Clone, PartialEq)]
pub struct Index {
    /// The index name.
    pub name: &'static str,
    /// The indexed table.
    pub table: TableRef,
    /// The indexed columns, in order.
    pub columns: Vec<&'static str>,
    /// Whether indexed values must be unique.
    pub unique: bool,
}

#[cfg(test)]
mod tests;
