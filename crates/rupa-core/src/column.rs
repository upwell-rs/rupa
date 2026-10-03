//! Column kinds, codecs and typed column handles.
//!
//! A column's kind (scalar or JSON) is part of its handle's type:
//! `Column<User, Prefs, Json>`. JSON-only operations bound on `K: IsJson`, so
//! using them on a scalar column fails to compile.
//!
//! Hand-written entities construct handles with [`Column::scalar`] /
//! [`Column::json`] / [`Column::nullable_json`], which check that the field type
//! can be stored that way. `#[derive(Entity)]` instead infers the kind per
//! field (autoref specialization in `__private`); since that only works in
//! expression context, derived handles are obtained with [`col!`].

use std::fmt;
use std::marker::PhantomData;

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{DecodeError, ResultError};
use crate::ir::ColumnRef;
use crate::row::Row;
use crate::value::{ScalarColumn, SqlType, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ColumnKind {
    Scalar,
    Json,
}

/// Static description of one entity column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnMeta {
    pub field: &'static str,
    pub name: &'static str,
    pub kind: ColumnKind,
    pub sql_type: SqlType,
    pub nullable: bool,
}

#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be stored in a column",
    note = "a column type must implement `ScalarColumn` (scalar column) or `Serialize + DeserializeOwned` (JSON column)"
)]
pub trait JsonColumn: Serialize + DeserializeOwned + 'static {}
impl<T: Serialize + DeserializeOwned + 'static> JsonColumn for T {}

/// Type-level column kinds.
#[derive(Debug, Clone, Copy)]
pub struct Scalar;
#[derive(Debug, Clone, Copy)]
pub struct Json;

pub trait KindMarker: 'static {
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
pub trait IsJson {}
impl IsJson for Json {}

/// Converts a field's Rust value to and from a [`Value`].
pub trait Codec<T> {
    type Kind: KindMarker;
    const SQL_TYPE: SqlType;
    const NULLABLE: bool;
    fn encode(v: &T) -> Value;
    fn decode(v: Value) -> Result<T, DecodeError>;
}

pub struct ScalarCodec<T>(PhantomData<fn() -> T>);
pub struct JsonCodec<T>(PhantomData<fn() -> T>);
/// JSON codec for `Option<T>`: `None` is SQL `NULL`, not JSON `null`.
pub struct NullableJsonCodec<T>(PhantomData<fn() -> T>);

macro_rules! zst_new {
    ($($t:ident),*) => {$(
        impl<T> $t<T> {
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
    ColumnMeta {
        field,
        name,
        kind: <C::Kind as KindMarker>::KIND,
        sql_type: C::SQL_TYPE,
        nullable: C::NULLABLE,
    }
}

/// Typed handle to column `T` of entity `E`, with kind `K` ([`Scalar`] or [`Json`]).
pub struct Column<E, T, K> {
    name: &'static str,
    sql_type: SqlType,
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
            .field("sql_type", &self.sql_type)
            .finish()
    }
}

impl<E, T, K> Column<E, T, K> {
    const fn with_codec<C: Codec<T, Kind = K>>(name: &'static str) -> Self {
        Self {
            name,
            sql_type: C::SQL_TYPE,
            encode: C::encode,
            decode: C::decode,
            _p: PhantomData,
        }
    }

    pub const fn name(&self) -> &'static str {
        self.name
    }

    pub const fn sql_type(&self) -> SqlType {
        self.sql_type
    }

    pub const fn column_ref(&self) -> ColumnRef {
        ColumnRef::new(self.name)
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

    pub fn kind(&self) -> ColumnKind
    where
        K: KindMarker,
    {
        K::KIND
    }
}

impl<E, T: ScalarColumn> Column<E, T, Scalar> {
    pub const fn scalar(name: &'static str) -> Self {
        Self::with_codec::<ScalarCodec<T>>(name)
    }
}

impl<E, T: JsonColumn> Column<E, T, Json> {
    pub const fn json(name: &'static str) -> Self {
        Self::with_codec::<JsonCodec<T>>(name)
    }
}

impl<E, T: JsonColumn> Column<E, Option<T>, Json> {
    pub const fn nullable_json(name: &'static str) -> Self {
        Self::with_codec::<NullableJsonCodec<T>>(name)
    }
}

/// `col!(Entity::field)`: the typed handle for a derived entity's field, with
/// its inferred kind. Expression context only.
#[macro_export]
macro_rules! col {
    ($entity:ident :: $field:ident) => {{
        #[allow(unused_imports)]
        use $crate::__private::{JsonLevel as _, NullableJsonLevel as _, ScalarLevel as _};
        let probe = <$entity>::__rupa_fields().$field;
        $crate::__private::column_from_probe(&probe, (&&&probe).__rupa_codec())
    }};
}

#[doc(hidden)]
pub mod __private {
    //! Autoref specialization used by `#[derive(Entity)]` and [`col!`]. Method
    //! resolution on `(&&&Probe<E, T>).__rupa_codec()` picks, highest first:
    //! 1. `T: ScalarColumn` -> `ScalarCodec<T>`
    //! 2. `T = Option<U>`, `U: JsonColumn` -> `NullableJsonCodec<U>`
    //! 3. otherwise -> `JsonCodec<T>`, with `T: JsonColumn` required on the
    //!    *method* so failures report our diagnostic.
    //!
    //! Resolution happens where the call is written. Inside generic code it
    //! follows the bounds in scope, which is why the derive refuses to infer
    //! kinds for fields whose type mentions a generic parameter.

    use super::*;

    pub struct Probe<E, T> {
        name: &'static str,
        _p: PhantomData<fn() -> (E, T)>,
    }

    impl<E, T> Clone for Probe<E, T> {
        fn clone(&self) -> Self {
            *self
        }
    }
    impl<E, T> Copy for Probe<E, T> {}

    impl<E, T> Probe<E, T> {
        pub const fn new(name: &'static str) -> Self {
            Self {
                name,
                _p: PhantomData,
            }
        }
        pub const fn name(&self) -> &'static str {
            self.name
        }
    }

    pub trait ScalarLevel<T> {
        fn __rupa_codec(&self) -> ScalarCodec<T>;
    }
    impl<E, T: ScalarColumn> ScalarLevel<T> for &&Probe<E, T> {
        fn __rupa_codec(&self) -> ScalarCodec<T> {
            ScalarCodec::new()
        }
    }

    pub trait NullableJsonLevel<T> {
        fn __rupa_codec(&self) -> NullableJsonCodec<T>;
    }
    impl<E, T: JsonColumn> NullableJsonLevel<T> for &Probe<E, Option<T>> {
        fn __rupa_codec(&self) -> NullableJsonCodec<T> {
            NullableJsonCodec::new()
        }
    }

    pub trait JsonLevel<T> {
        fn __rupa_codec(&self) -> JsonCodec<T>
        where
            T: JsonColumn;
    }
    impl<E, T> JsonLevel<T> for Probe<E, T> {
        fn __rupa_codec(&self) -> JsonCodec<T>
        where
            T: JsonColumn,
        {
            JsonCodec::new()
        }
    }

    pub fn column_from_probe<E, T, C: Codec<T>>(
        probe: &Probe<E, T>,
        _codec: C,
    ) -> Column<E, T, C::Kind> {
        Column::with_codec::<C>(probe.name())
    }

    pub fn meta_from_codec<T, C: Codec<T>>(
        field: &'static str,
        name: &'static str,
        _codec: &C,
    ) -> ColumnMeta {
        column_meta::<T, C>(field, name)
    }
}
