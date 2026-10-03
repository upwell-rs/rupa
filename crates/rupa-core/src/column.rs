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
    /// The database supplies this column's value (identity, default). Skipped
    /// by entity-level inserts and updates.
    pub generated: bool,
}

impl ColumnMeta {
    pub const fn generated(mut self) -> Self {
        self.generated = true;
        self
    }
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
        generated: false,
    }
}

/// Typed handle to column `T` of entity `E`, with kind `K` ([`Scalar`] or [`Json`]).
pub struct Column<E, T, K> {
    name: &'static str,
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
            .field("sql_type", &self.sql_type)
            .finish()
    }
}

impl<E, T, K> Column<E, T, K> {
    const fn with_codec<C: Codec<T, Kind = K>>(name: &'static str) -> Self {
        Self {
            name,
            sql_type: C::SQL_TYPE,
            nullable: C::NULLABLE,
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

    /// Column metadata for the entity field named `field`.
    pub fn meta(&self, field: &'static str) -> ColumnMeta
    where
        K: KindMarker,
    {
        ColumnMeta {
            field,
            name: self.name,
            kind: K::KIND,
            sql_type: self.sql_type,
            nullable: self.nullable,
            generated: false,
        }
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
/// its kind (inferred, or as overridden by `#[column(scalar|json)]`).
/// Expression context only.
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
    //! Field probes used by `#[derive(Entity)]`, the capability derives and
    //! [`col!`]. A derived entity exposes one probe per field; calling
    //! `(&&&probe).__rupa_codec()` yields the field's codec.
    //!
    //! For an inferring [`Probe`], method resolution picks (highest first):
    //! 1. `T: ScalarColumn` -> `ScalarCodec<T>`
    //! 2. `T = Option<U>`, `U: JsonColumn` -> `NullableJsonCodec<U>`
    //! 3. otherwise -> `JsonCodec<T>`, with `T: JsonColumn` required on the
    //!    *method* so failures report our diagnostic.
    //!
    //! Fields with an explicit `#[column(scalar|json)]` get a [`ScalarProbe`],
    //! [`JsonProbe`] or [`NullableJsonProbe`] instead. Their inherent
    //! `__rupa_codec` is the only candidate, so the override always wins.
    //!
    //! Resolution happens where the call is written, so it is only meaningful
    //! for concrete types; the derive rejects generic entities.

    use super::*;

    /// A probe for one field of entity `E`.
    pub trait FieldProbe: Copy {
        type Entity;
        type Field;
        fn name(&self) -> &'static str;
    }

    macro_rules! probe {
        ($(#[$m:meta])* $name:ident<$e:ident, $t:ident> => $field:ty) => {
            $(#[$m])*
            pub struct $name<$e, $t> {
                name: &'static str,
                _p: PhantomData<fn() -> ($e, $t)>,
            }
            impl<$e, $t> Clone for $name<$e, $t> {
                fn clone(&self) -> Self {
                    *self
                }
            }
            impl<$e, $t> Copy for $name<$e, $t> {}
            impl<$e, $t> $name<$e, $t> {
                pub const fn new(name: &'static str) -> Self {
                    Self { name, _p: PhantomData }
                }
            }
            impl<$e, $t> FieldProbe for $name<$e, $t> {
                type Entity = $e;
                type Field = $field;
                fn name(&self) -> &'static str {
                    self.name
                }
            }
        };
    }

    probe!(
        /// Infers the kind from the field type.
        Probe<E, T> => T
    );
    probe!(
        /// `#[column(scalar)]`
        ScalarProbe<E, T> => T
    );
    probe!(
        /// `#[column(json)]` on a non-`Option` field.
        JsonProbe<E, T> => T
    );
    probe!(
        /// `#[column(json)]` on an `Option<U>` field; `None` is SQL `NULL`.
        NullableJsonProbe<E, U> => Option<U>
    );

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

    impl<E, T: ScalarColumn> ScalarProbe<E, T> {
        pub fn __rupa_codec(&self) -> ScalarCodec<T> {
            ScalarCodec::new()
        }
    }
    impl<E, T: JsonColumn> JsonProbe<E, T> {
        pub fn __rupa_codec(&self) -> JsonCodec<T> {
            JsonCodec::new()
        }
    }
    impl<E, U: JsonColumn> NullableJsonProbe<E, U> {
        pub fn __rupa_codec(&self) -> NullableJsonCodec<U> {
            NullableJsonCodec::new()
        }
    }

    pub fn column_from_probe<P: FieldProbe, C: Codec<P::Field>>(
        probe: &P,
        _codec: C,
    ) -> Column<P::Entity, P::Field, C::Kind> {
        Column::with_codec::<C>(probe.name())
    }

    /// Compile-time check that a companion struct's field of type `T` matches
    /// the entity field this probe describes. Implemented per probe type (not
    /// via `FieldProbe::Field`) so a mismatch reports this diagnostic.
    #[diagnostic::on_unimplemented(
        message = "this field's type `{T}` does not match the entity's field",
        label = "expected the entity field's type",
        note = "the entity field is described by `{Self}`"
    )]
    pub trait FieldIs<T> {
        /// The entity field type (always `T` when implemented).
        type Field;
        fn cast(value: &T) -> &Self::Field;
    }
    impl<E, T> FieldIs<T> for Probe<E, T> {
        type Field = T;
        fn cast(value: &T) -> &T {
            value
        }
    }
    impl<E, T> FieldIs<T> for ScalarProbe<E, T> {
        type Field = T;
        fn cast(value: &T) -> &T {
            value
        }
    }
    impl<E, T> FieldIs<T> for JsonProbe<E, T> {
        type Field = T;
        fn cast(value: &T) -> &T {
            value
        }
    }
    impl<E, U> FieldIs<Option<U>> for NullableJsonProbe<E, U> {
        type Field = Option<U>;
        fn cast(value: &Option<U>) -> &Option<U> {
            value
        }
    }

    pub fn assert_field_type<T, P: FieldIs<T>>(_probe: &P) {}

    /// `value` as the entity field type, checked by [`FieldIs`]. When the check
    /// fails, only that diagnostic is reported (not a second type mismatch).
    pub fn as_field<'a, T, P: FieldIs<T>>(_probe: &P, value: &'a T) -> &'a P::Field {
        P::cast(value)
    }

    const fn str_eq(a: &str, b: &str) -> bool {
        let (a, b) = (a.as_bytes(), b.as_bytes());
        if a.len() != b.len() {
            return false;
        }
        let mut i = 0;
        while i < a.len() {
            if a[i] != b[i] {
                return false;
            }
            i += 1;
        }
        true
    }

    /// Const check that an `Insertable` companion provides every field the
    /// entity requires on insert. `required` pairs a field with its error message.
    pub const fn assert_insert_covers(required: &[(&str, &str)], provided: &[&str]) {
        let mut i = 0;
        while i < required.len() {
            let (field, message) = required[i];
            let mut found = false;
            let mut j = 0;
            while j < provided.len() {
                if str_eq(field, provided[j]) {
                    found = true;
                }
                j += 1;
            }
            if !found {
                panic!("{}", message);
            }
            i += 1;
        }
    }

    /// Const check that a patch's `#[id]` field is the entity's id field.
    pub const fn assert_id_field(entity_id: &str, patch_id: &str, message: &str) {
        if !str_eq(entity_id, patch_id) {
            panic!("{}", message);
        }
    }
}
