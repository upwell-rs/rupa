use crate::column::ColumnMeta;
use crate::error::ResultError;
use crate::ir::TableRef;
use crate::row::Row;
use crate::value::{ScalarColumn, Value};

/// Decodes one row, by position, into `Self`.
pub trait FromRow: Sized {
    /// Decodes `row`, whose columns are in projection order.
    fn from_row(row: &dyn Row) -> Result<Self, ResultError>;
}

/// An entity id, as the values of its id columns (in `Entity::ID_COLUMNS` order).
/// Only single scalar ids are implemented; composite ids would add tuple impls.
pub trait IdValues {
    /// The id column values, in `Entity::ID_COLUMNS` order.
    fn id_values(&self) -> Vec<Value>;
}

impl<T: ScalarColumn> IdValues for T {
    fn id_values(&self) -> Vec<Value> {
        vec![self.to_value()]
    }
}

/// A table-backed type. Implementing `Entity` grants no insert, update or
/// delete surface; those are separate capability traits.
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not an entity",
    note = "derive or implement `Entity` for it"
)]
pub trait Entity: FromRow + 'static {
    /// The id type: a scalar for single-column ids.
    type Id: IdValues;
    /// The table, with its schema if any.
    const TABLE: TableRef;
    /// Id column names. A slice so that composite ids need no trait change.
    const ID_COLUMNS: &'static [&'static str];
    /// All columns, in the order used for projection, `to_values` and `from_row`.
    const COLUMNS: &'static [ColumnMeta];

    /// This entity's id.
    fn id(&self) -> &Self::Id;

    /// Field values in `COLUMNS` order.
    fn to_values(&self) -> Vec<Value>;
}
