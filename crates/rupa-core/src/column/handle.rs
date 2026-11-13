//! The typed column handle and reading columns into fields.

use std::fmt;
use std::marker::PhantomData;

use super::{
    Codec, ColumnMeta, Encoding, Json, JsonCodec, JsonColumn, KindMarker, NullableJsonCodec,
    Scalar, ScalarCodec, StorageKind,
};
use crate::error::{DecodeError, ResultError};
use crate::ir::{ColumnRef, SourceId, TableRef};
use crate::row::Row;
use crate::schema::ColumnType;
use crate::value::{ScalarColumn, SqlType, Value};

/// Typed handle to column `T` of entity `E`, with kind `K` ([`Scalar`] or [`Json`]).
///
/// A handle refers to the statement's first row source unless it came from a
/// join's source handle ([`Src::col`](crate::join::Src::col)).
pub struct Column<E, T, K> {
    name: &'static str,
    table: Option<TableRef>,
    source: SourceId,
    sql_type: SqlType,
    column_type: ColumnType,
    nullable: bool,
    encode: fn(&T) -> Value,
    decode: fn(Value) -> Result<T, DecodeError>,
    _p: PhantomData<fn() -> (E, K)>,
}

impl<E, T, K> Clone for Column<E, T, K> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<E, T, K> Copy for Column<E, T, K> {}

impl<E, T, K> fmt::Debug for Column<E, T, K> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Column")
            .field("name", &self.name)
            .field("source", &self.source.0)
            .field("sql_type", &self.sql_type)
            .finish()
    }
}

impl<E, T, K> Column<E, T, K> {
    const fn with_codec<C: Codec<T, Kind = K>>(name: &'static str) -> Self {
        Self {
            name,
            table: None,
            source: SourceId(0),
            sql_type: C::SQL_TYPE,
            column_type: C::COLUMN_TYPE,
            nullable: C::NULLABLE,
            encode: C::encode,
            decode: C::decode,
            _p: PhantomData,
        }
    }

    /// The column name in the database.
    pub const fn name(&self) -> &'static str {
        self.name
    }

    /// The SQL type values are bound and decoded as.
    pub const fn sql_type(&self) -> SqlType {
        self.sql_type
    }

    /// The column type a schema declares, with its length for bounded text.
    pub const fn column_type(&self) -> ColumnType {
        self.column_type
    }

    /// Whether the column admits `NULL`.
    pub const fn nullable(&self) -> bool {
        self.nullable
    }

    /// The table the handle's entity maps to, if the handle was built for it
    /// (`#[derive(Entity)]` does, and so do the `_in` constructors).
    pub const fn table(&self) -> Option<TableRef> {
        self.table
    }

    /// The row source this handle refers to.
    pub const fn source(&self) -> SourceId {
        self.source
    }

    /// The same column, of row source `source`. Join source handles do this
    /// and check the entity; see [`Src::col`](crate::join::Src::col).
    pub const fn of_source(mut self, source: SourceId) -> Self {
        self.source = source;
        self
    }

    /// The same column, recorded as belonging to `table`.
    pub const fn in_table(mut self, table: TableRef) -> Self {
        self.table = Some(table);

        self
    }

    /// The column as an IR reference, carrying its table when the handle has
    /// one.
    pub const fn column_ref(&self) -> ColumnRef {
        let column = ColumnRef::of(self.source, self.name);

        match self.table {
            Some(table) => column.in_table(table),
            None => column,
        }
    }

    /// Encodes a field value exactly as the entity stores it.
    pub fn encode(&self, v: &T) -> Value {
        (self.encode)(v)
    }

    /// Reads and decodes this column from position `index` of `row`.
    pub fn read(&self, row: &dyn Row, index: usize) -> Result<T, ResultError> {
        let v = row.get(index, self.sql_type)?;
        (self.decode)(v).map_err(|e| e.in_column(self.name).into())
    }

    /// Scalar or JSON.
    pub const fn kind(&self) -> StorageKind
    where
        K: KindMarker,
    {
        K::KIND
    }

    /// Column metadata for the entity field named `field`.
    pub const fn meta(&self, field: &'static str) -> ColumnMeta
    where
        K: KindMarker,
    {
        ColumnMeta::new(field, self.name, K::KIND, self.sql_type, self.nullable)
            .column_type(self.column_type)
    }
}

impl<E, T, K: KindMarker> Column<E, T, K> {
    /// A handle for column `name` of `table`, stored with `encoding`.
    pub const fn with_encoding_in(
        table: TableRef,
        name: &'static str,
        encoding: Encoding<T>,
    ) -> Self {
        Self::with_encoding(name, encoding).in_table(table)
    }

    /// A handle for column `name`, of no known table, stored with
    /// `encoding`. Fails to compile in a constant when the encoding's kind
    /// is not `K`.
    pub const fn with_encoding(name: &'static str, encoding: Encoding<T>) -> Self {
        assert!(
            matches!(
                (K::KIND, encoding.kind),
                (StorageKind::Scalar, StorageKind::Scalar) | (StorageKind::Json, StorageKind::Json)
            ),
            "the column kind does not match its encoding"
        );

        Self {
            name,
            table: None,
            source: SourceId(0),
            sql_type: encoding.sql_type,
            column_type: encoding.column_type,
            nullable: encoding.nullable,
            encode: encoding.encode,
            decode: encoding.decode,
            _p: PhantomData,
        }
    }
}

impl<E, T: ScalarColumn> Column<E, T, Scalar> {
    /// A handle for a scalar column named `name`, of no known table.
    pub const fn scalar(name: &'static str) -> Self {
        Self::with_codec::<ScalarCodec<T>>(name)
    }

    /// A handle for a scalar column named `name` of `table`.
    pub const fn scalar_in(table: TableRef, name: &'static str) -> Self {
        Self::scalar(name).in_table(table)
    }
}

impl<E, T: JsonColumn> Column<E, T, Json> {
    /// A handle for a JSON column named `name`, of no known table.
    pub const fn json(name: &'static str) -> Self {
        Self::with_codec::<JsonCodec<T>>(name)
    }

    /// A handle for a JSON column named `name` of `table`.
    pub const fn json_in(table: TableRef, name: &'static str) -> Self {
        Self::json(name).in_table(table)
    }
}

impl<E, T: JsonColumn> Column<E, Option<T>, Json> {
    /// A handle for a nullable JSON column: `None` is SQL `NULL`. Of no
    /// known table.
    pub const fn nullable_json(name: &'static str) -> Self {
        Self::with_codec::<NullableJsonCodec<T>>(name)
    }

    /// A handle for a nullable JSON column named `name` of `table`.
    pub const fn nullable_json_in(table: TableRef, name: &'static str) -> Self {
        Self::nullable_json(name).in_table(table)
    }
}

/// Reads a column into a field of type `F`: the column's own type `T`, or
/// `Option<T>`, where `NULL` is `None` (a column of a left-joined entity).
/// Used by `#[derive(Joined)]`.
#[diagnostic::on_unimplemented(
    message = "this field can't hold the column `{Self}`",
    note = "a `#[from(..)]` field has its column's type, or an `Option` of it"
)]
pub trait ReadAs<F> {
    /// Reads and decodes position `index` of `row`.
    fn read_as(&self, row: &dyn Row, index: usize) -> Result<F, ResultError>;
}

impl<E, T, K> ReadAs<T> for Column<E, T, K> {
    fn read_as(&self, row: &dyn Row, index: usize) -> Result<T, ResultError> {
        self.read(row, index)
    }
}

impl<E, T, K> ReadAs<Option<T>> for Column<E, T, K> {
    fn read_as(&self, row: &dyn Row, index: usize) -> Result<Option<T>, ResultError> {
        match row.get(index, self.sql_type)? {
            Value::Null(_) => Ok(None),
            v => (self.decode)(v)
                .map(Some)
                .map_err(|e| e.in_column(self.name).into()),
        }
    }
}
