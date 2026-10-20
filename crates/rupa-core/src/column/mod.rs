//! Column kinds, codecs and typed column handles.
//!
//! A column's kind (scalar or JSON) is part of its handle's type:
//! `Column<User, Prefs, Json>`. JSON-only operations bound on `K: IsJson`, so
//! using them on a scalar column fails to compile.
//!
//! Hand-written entities construct handles with [`Column::scalar`] /
//! [`Column::json`] / [`Column::nullable_json`], which check that the field type
//! can be stored that way. `#[derive(Entity)]` emits one handle constant per
//! field (`User::EMAIL`) and infers its kind at compile time: scalar for
//! [`ScalarColumn`] types, nullable JSON for `Option<U>` of a serde type, JSON
//! for any other serde type (see `__private`).

use std::fmt;
use std::marker::PhantomData;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{DecodeError, ResultError};
use crate::ir::{ColumnRef, SourceId, TableRef};
use crate::row::Row;
use crate::value::{ScalarColumn, SqlType, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// How a column is stored.
pub enum ColumnKind {
    /// A plain SQL value (`ScalarColumn`).
    Scalar,
    /// A JSON document (any `Serialize + DeserializeOwned` type).
    Json,
}

/// How the database supplies a column's value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Generated {
    /// An integer identity (auto-increment) key: `#[id(generated)]`.
    Identity,
    /// A database default, trigger or computed value: `#[column(generated)]`.
    Database,
}

/// The column a foreign key points at.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Reference {
    /// The referenced table.
    pub table: TableRef,
    /// The referenced columns: the target entity's id columns.
    pub columns: &'static [&'static str],
}

/// Static description of one entity column: everything a schema needs to
/// know about it, available as a constant (`Entity::COLUMNS`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnMeta {
    /// The Rust field name.
    pub field: &'static str,
    /// The column name in the database.
    pub name: &'static str,
    /// Scalar or JSON.
    pub kind: ColumnKind,
    /// The SQL type values are bound and decoded as.
    pub sql_type: SqlType,
    /// Whether the column admits `NULL` (an `Option` field).
    pub nullable: bool,
    /// Part of the primary key (`#[id]`).
    pub primary_key: bool,
    /// The database supplies this column's value. Skipped by entity-level
    /// inserts and updates.
    pub generated: Option<Generated>,
    /// The foreign key this column holds (`#[column(references = Entity)]`).
    pub references: Option<Reference>,
}

impl ColumnMeta {
    /// A plain column: not a key, not generated, referencing nothing.
    pub const fn new(
        field: &'static str,
        name: &'static str,
        kind: ColumnKind,
        sql_type: SqlType,
        nullable: bool,
    ) -> Self {
        Self {
            field,
            name,
            kind,
            sql_type,
            nullable,
            primary_key: false,
            generated: None,
            references: None,
        }
    }

    /// Marks the column as part of the primary key.
    pub const fn primary_key(mut self) -> Self {
        self.primary_key = true;

        self
    }

    /// Marks the column as supplied by the database.
    pub const fn generated(mut self, generated: Generated) -> Self {
        self.generated = Some(generated);

        self
    }

    /// Marks the column as a foreign key to `reference`.
    pub const fn references(mut self, reference: Reference) -> Self {
        self.references = Some(reference);

        self
    }
}

#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be stored in a column",
    note = "a column type must implement `ScalarColumn` (scalar column) or `Serialize + DeserializeOwned` (JSON column)"
)]
/// Types that can be stored as a JSON column: anything serde can
/// round-trip.
pub trait JsonColumn: Serialize + DeserializeOwned + 'static {}
impl<T: Serialize + DeserializeOwned + 'static> JsonColumn for T {}

/// Type-level kind of scalar columns.
#[derive(Debug, Clone, Copy)]
pub struct Scalar;

/// Type-level kind of JSON columns.
#[derive(Debug, Clone, Copy)]
pub struct Json;

/// Selects a type-level kind from a constant, so `#[derive(Entity)]` can name
/// the kind it inferred at compile time: `<Stored<false> as Kinded>::Kind` is
/// [`Scalar`], `<Stored<true> as Kinded>::Kind` is [`Json`].
#[derive(Debug, Clone, Copy)]
pub struct Stored<const JSON: bool>;

/// Maps [`Stored`] to its type-level kind.
pub trait Kinded {
    /// [`Scalar`] or [`Json`].
    type Kind: KindMarker;
}

impl Kinded for Stored<false> {
    type Kind = Scalar;
}

impl Kinded for Stored<true> {
    type Kind = Json;
}

/// Maps a type-level kind ([`Scalar`], [`Json`]) to its [`ColumnKind`].
pub trait KindMarker: 'static {
    /// The runtime kind.
    const KIND: ColumnKind;
}

impl KindMarker for Scalar {
    const KIND: ColumnKind = ColumnKind::Scalar;
}

impl KindMarker for Json {
    const KIND: ColumnKind = ColumnKind::Json;
}

#[diagnostic::on_unimplemented(
    message = "JSON path access requires a JSON column, but this column's kind is `{Self}`",
    label = "not a JSON column"
)]
/// Implemented by kinds that support JSON operations (path access,
/// containment).
pub trait IsJson {}
impl IsJson for Json {}

/// Converts a field's Rust value to and from a [`Value`].
pub trait Codec<T> {
    /// The column kind this codec stores.
    type Kind: KindMarker;
    /// The SQL type values are bound and decoded as.
    const SQL_TYPE: SqlType;
    /// Whether `NULL` is a valid value.
    const NULLABLE: bool;
    /// Encodes a field value.
    fn encode(v: &T) -> Value;
    /// Decodes a field value.
    fn decode(v: Value) -> Result<T, DecodeError>;
}

/// Codec for [`ScalarColumn`] types.
pub struct ScalarCodec<T>(PhantomData<fn() -> T>);
/// Codec for JSON columns: the value is (de)serialized with serde.
pub struct JsonCodec<T>(PhantomData<fn() -> T>);
/// JSON codec for `Option<T>`: `None` is SQL `NULL`, not JSON `null`.
pub struct NullableJsonCodec<T>(PhantomData<fn() -> T>);

macro_rules! zst_new {
    ($($t:ident),*) => {$(
        impl<T> $t<T> {
            /// The codec (a zero-sized value).
            pub const fn new() -> Self {
                Self(PhantomData)
            }
        }
        impl<T> Default for $t<T> {
            fn default() -> Self {
                Self::new()
            }
        }
    )*};
}
zst_new!(ScalarCodec, JsonCodec, NullableJsonCodec);

impl<T: ScalarColumn> Codec<T> for ScalarCodec<T> {
    type Kind = Scalar;
    const SQL_TYPE: SqlType = T::SQL_TYPE;
    const NULLABLE: bool = T::NULLABLE;
    fn encode(v: &T) -> Value {
        v.to_value()
    }
    fn decode(v: Value) -> Result<T, DecodeError> {
        T::from_value(v)
    }
}

impl<T: JsonColumn> Codec<T> for JsonCodec<T> {
    type Kind = Json;
    const SQL_TYPE: SqlType = SqlType::Json;
    const NULLABLE: bool = false;
    fn encode(v: &T) -> Value {
        // Serialization into `serde_json::Value` fails only for maps with
        // non-string keys or a failing custom `Serialize` impl: a programming error.
        Value::Json(serde_json::to_value(v).expect("JSON column value failed to serialize"))
    }
    fn decode(v: Value) -> Result<T, DecodeError> {
        match v {
            Value::Json(j) => {
                serde_json::from_value(j).map_err(|e| DecodeError::new(e.to_string()))
            }
            other => Err(DecodeError::type_mismatch(SqlType::Json, &other)),
        }
    }
}

impl<T: JsonColumn> Codec<Option<T>> for NullableJsonCodec<T> {
    type Kind = Json;
    const SQL_TYPE: SqlType = SqlType::Json;
    const NULLABLE: bool = true;
    fn encode(v: &Option<T>) -> Value {
        match v {
            Some(v) => JsonCodec::<T>::encode(v),
            None => Value::Null(SqlType::Json),
        }
    }
    fn decode(v: Value) -> Result<Option<T>, DecodeError> {
        match v {
            Value::Null(_) => Ok(None),
            v => JsonCodec::<T>::decode(v).map(Some),
        }
    }
}

/// Builds the [`ColumnMeta`] for a field stored through codec `C`.
pub const fn column_meta<T, C: Codec<T>>(field: &'static str, name: &'static str) -> ColumnMeta {
    ColumnMeta::new(
        field,
        name,
        <C::Kind as KindMarker>::KIND,
        C::SQL_TYPE,
        C::NULLABLE,
    )
}

/// How values of `T` are stored, as a constant: the codec `#[derive(Entity)]`
/// selected for a field, with its kind.
pub struct Encoding<T> {
    kind: ColumnKind,
    sql_type: SqlType,
    nullable: bool,
    encode: fn(&T) -> Value,
    decode: fn(Value) -> Result<T, DecodeError>,
}

impl<T> Clone for Encoding<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Encoding<T> {}

impl<T> Encoding<T> {
    /// The encoding of codec `C`.
    pub const fn of<C: Codec<T>>() -> Self {
        Self {
            kind: <C::Kind as KindMarker>::KIND,
            sql_type: C::SQL_TYPE,
            nullable: C::NULLABLE,
            encode: C::encode,
            decode: C::decode,
        }
    }

    /// Whether values are stored as JSON documents.
    pub const fn is_json(&self) -> bool {
        matches!(self.kind, ColumnKind::Json)
    }
}

/// Typed handle to column `T` of entity `E`, with kind `K` ([`Scalar`] or [`Json`]).
///
/// A handle refers to the statement's first row source unless it came from a
/// join's source handle ([`Src::col`](crate::join::Src::col)).
pub struct Column<E, T, K> {
    name: &'static str,
    source: SourceId,
    sql_type: SqlType,
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
            source: SourceId(0),
            sql_type: C::SQL_TYPE,
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

    /// Whether the column admits `NULL`.
    pub const fn nullable(&self) -> bool {
        self.nullable
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

    /// The column as an IR reference.
    pub const fn column_ref(&self) -> ColumnRef {
        ColumnRef::of(self.source, self.name)
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
    pub const fn kind(&self) -> ColumnKind
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
    }
}

impl<E, T, K: KindMarker> Column<E, T, K> {
    /// A handle for column `name`, stored with `encoding`. Fails to compile
    /// in a constant when the encoding's kind is not `K`.
    pub const fn with_encoding(name: &'static str, encoding: Encoding<T>) -> Self {
        assert!(
            matches!(
                (K::KIND, encoding.kind),
                (ColumnKind::Scalar, ColumnKind::Scalar) | (ColumnKind::Json, ColumnKind::Json)
            ),
            "the column kind does not match its encoding"
        );

        Self {
            name,
            source: SourceId(0),
            sql_type: encoding.sql_type,
            nullable: encoding.nullable,
            encode: encoding.encode,
            decode: encoding.decode,
            _p: PhantomData,
        }
    }
}

impl<E, T: ScalarColumn> Column<E, T, Scalar> {
    /// A handle for a scalar column named `name`.
    pub const fn scalar(name: &'static str) -> Self {
        Self::with_codec::<ScalarCodec<T>>(name)
    }
}

impl<E, T: JsonColumn> Column<E, T, Json> {
    /// A handle for a JSON column named `name`.
    pub const fn json(name: &'static str) -> Self {
        Self::with_codec::<JsonCodec<T>>(name)
    }
}

impl<E, T: JsonColumn> Column<E, Option<T>, Json> {
    /// A handle for a nullable JSON column: `None` is SQL `NULL`.
    pub const fn nullable_json(name: &'static str) -> Self {
        Self::with_codec::<NullableJsonCodec<T>>(name)
    }
}

#[doc(hidden)]
pub mod __private;

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
