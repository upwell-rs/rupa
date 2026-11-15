//! Columns of a table known only by its name.

use super::{Column, Json, JsonColumn, Scalar};
use crate::ir::TableRef;
use crate::value::ScalarColumn;

/// The entity of a [`Column`] built with [`Column::dynamic`] and its siblings:
/// a column of a table no Rust type declares, such as one only a migration
/// knows. The value type is checked by the compiler; the table is checked
/// when the schema is rendered, because the handle records it. A dynamic
/// column of table `X` named in an index or check on table `Y` is refused
/// there.
///
/// It is not a [`Relation`](crate::Relation): statements that name the
/// relation of their columns (`update::<R>().set(..)`) do not take it.
#[derive(Debug, Clone, Copy)]
pub struct Dynamic;

impl Column<Dynamic, (), Scalar> {
    /// A handle for the scalar column `name` of `table`, holding `T`:
    /// `Column::dynamic::<i64>(accounts, "age")`.
    pub const fn dynamic<T: ScalarColumn>(
        table: TableRef,
        name: &'static str,
    ) -> Column<Dynamic, T, Scalar> {
        Column::scalar_in(table, name)
    }
}

impl Column<Dynamic, (), Json> {
    /// A handle for the JSON column `name` of `table`, holding `T`.
    pub const fn dynamic_json<T: JsonColumn>(
        table: TableRef,
        name: &'static str,
    ) -> Column<Dynamic, T, Json> {
        Column::json_in(table, name)
    }

    /// A handle for the nullable JSON column `name` of `table`: `None` is
    /// SQL `NULL`.
    pub const fn dynamic_nullable_json<T: JsonColumn>(
        table: TableRef,
        name: &'static str,
    ) -> Column<Dynamic, Option<T>, Json> {
        Column::nullable_json_in(table, name)
    }
}

impl<T, K> Column<Dynamic, T, K> {
    /// The table the handle was declared for. Every dynamic handle has one:
    /// [`Column::dynamic`] and its siblings record it, and the table-less
    /// constructors (`Column::scalar`, `json`, `nullable_json`,
    /// `with_encoding`) require a [`Relation`](crate::Relation), which
    /// [`Dynamic`] is not.
    pub const fn declared_table(&self) -> TableRef {
        match self.table() {
            Some(table) => table,
            None => panic!("a dynamic column records its table"),
        }
    }
}
