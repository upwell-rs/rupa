//! Spike: infer scalar-vs-JSON column kind from a field's type, using autoref
//! specialization inside `#[derive(Entity)]` output.
//!
//! Priority order (highest first), selected by method resolution on
//! `(&&&Probe<E, T>).__rupa_codec()`:
//!
//! 1. `T: ScalarColumn`                       -> `ScalarCodec<T>`
//! 2. `T = Option<U>`, `U: Serialize + DeserializeOwned` -> `NullableJsonCodec<U>` (None => SQL NULL)
//! 3. anything else; requires `T: JsonColumn` -> `JsonCodec<T>`
//!
//! Level 3 is unconditional at the impl level and puts its requirement on the
//! *method*, so a type that is neither scalar nor serde gets our
//! `on_unimplemented` message instead of "no method named `__rupa_codec`".

#![allow(
    clippy::new_without_default,
    clippy::type_complexity,
    reason = "spike code"
)]

use std::marker::PhantomData;

use serde::Serialize;
use serde::de::DeserializeOwned;

pub use rupa_spike_column_kind_macros::Entity;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    I64(i64),
    F64(f64),
    Text(String),
    Bytes(Vec<u8>),
    Timestamp(chrono::DateTime<chrono::Utc>),
    Uuid(uuid::Uuid),
    Json(serde_json::Value),
}

#[derive(Debug)]
pub struct DecodeError(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnKind {
    Scalar,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ColumnMeta {
    pub field: &'static str,
    pub name: &'static str,
    pub kind: ColumnKind,
    pub nullable: bool,
}

pub trait Entity: Sized {
    const TABLE: &'static str;
    fn columns() -> Vec<ColumnMeta>;
    fn to_values(&self) -> Vec<Value>;
    fn from_values(values: Vec<Value>) -> Result<Self, DecodeError>;
}

// ---------------------------------------------------------------------------
// Scalar marker
// ---------------------------------------------------------------------------

#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a scalar column type",
    note = "implement `ScalarColumn` for it, or store it as JSON with `#[column(json)]`"
)]
pub trait ScalarColumn: Sized {
    const NULLABLE: bool = false;
    fn to_value(&self) -> Value;
    fn from_value(v: Value) -> Result<Self, DecodeError>;
}

macro_rules! scalar {
    ($t:ty, $variant:ident) => {
        impl ScalarColumn for $t {
            fn to_value(&self) -> Value {
                Value::$variant(self.clone())
            }
            fn from_value(v: Value) -> Result<Self, DecodeError> {
                match v {
                    Value::$variant(x) => Ok(x),
                    other => Err(DecodeError(format!(
                        concat!("expected ", stringify!($variant), ", got {:?}"),
                        other
                    ))),
                }
            }
        }
    };
}
scalar!(bool, Bool);
scalar!(i64, I64);
scalar!(f64, F64);
scalar!(String, Text);
scalar!(Vec<u8>, Bytes);
scalar!(chrono::DateTime<chrono::Utc>, Timestamp);
scalar!(uuid::Uuid, Uuid);

impl<T: ScalarColumn> ScalarColumn for Option<T> {
    const NULLABLE: bool = true;
    fn to_value(&self) -> Value {
        self.as_ref().map_or(Value::Null, T::to_value)
    }
    fn from_value(v: Value) -> Result<Self, DecodeError> {
        match v {
            Value::Null => Ok(None),
            v => T::from_value(v).map(Some),
        }
    }
}

#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be stored in a column",
    note = "a column type must implement `ScalarColumn` (scalar column) or `Serialize + DeserializeOwned` (inferred JSON column)"
)]
pub trait JsonColumn: Serialize + DeserializeOwned {}
impl<T: Serialize + DeserializeOwned> JsonColumn for T {}

// ---------------------------------------------------------------------------
// Codecs (value-level) and kinds (type-level)
// ---------------------------------------------------------------------------

pub struct Scalar;
pub struct Json;

pub trait KindMarker {
    const KIND: ColumnKind;
}
impl KindMarker for Scalar {
    const KIND: ColumnKind = ColumnKind::Scalar;
}
impl KindMarker for Json {
    const KIND: ColumnKind = ColumnKind::Json;
}

pub trait Codec<T> {
    type Kind: KindMarker;
    const NULLABLE: bool;
    fn encode(&self, v: &T) -> Value;
    fn decode(&self, v: Value) -> Result<T, DecodeError>;
}

pub struct ScalarCodec<T>(PhantomData<fn() -> T>);
pub struct JsonCodec<T>(PhantomData<fn() -> T>);
pub struct NullableJsonCodec<T>(PhantomData<fn() -> T>);

impl<T> ScalarCodec<T> {
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}
impl<T> JsonCodec<T> {
    pub const fn new() -> Self {
        Self(PhantomData)
    }
}

impl<T: ScalarColumn> Codec<T> for ScalarCodec<T> {
    type Kind = Scalar;
    const NULLABLE: bool = T::NULLABLE;
    fn encode(&self, v: &T) -> Value {
        v.to_value()
    }
    fn decode(&self, v: Value) -> Result<T, DecodeError> {
        T::from_value(v)
    }
}

impl<T: JsonColumn> Codec<T> for JsonCodec<T> {
    type Kind = Json;
    const NULLABLE: bool = false;
    fn encode(&self, v: &T) -> Value {
        Value::Json(serde_json::to_value(v).expect("JSON column serialization failed"))
    }
    fn decode(&self, v: Value) -> Result<T, DecodeError> {
        match v {
            Value::Json(j) => serde_json::from_value(j).map_err(|e| DecodeError(e.to_string())),
            other => Err(DecodeError(format!("expected Json, got {other:?}"))),
        }
    }
}

impl<T: JsonColumn> Codec<Option<T>> for NullableJsonCodec<T> {
    type Kind = Json;
    const NULLABLE: bool = true;
    fn encode(&self, v: &Option<T>) -> Value {
        v.as_ref()
            .map_or(Value::Null, |v| JsonCodec::<T>::new().encode(v))
    }
    fn decode(&self, v: Value) -> Result<Option<T>, DecodeError> {
        match v {
            Value::Null => Ok(None),
            v => JsonCodec::<T>::new().decode(v).map(Some),
        }
    }
}

// ---------------------------------------------------------------------------
// Typed column handles: kind is part of the type
// ---------------------------------------------------------------------------

pub struct Column<E, T, K> {
    name: &'static str,
    _p: PhantomData<fn() -> (E, T, K)>,
}

#[diagnostic::on_unimplemented(
    message = "JSON path access requires a JSON column, but this column's kind is `{Self}`",
    label = "not a JSON column"
)]
pub trait IsJson {}
impl IsJson for Json {}

#[derive(Debug, PartialEq)]
pub struct JsonPath {
    pub column: &'static str,
    pub path: Vec<&'static str>,
}

impl<E, T, K: KindMarker> Column<E, T, K> {
    pub fn from_probe<C: Codec<T, Kind = K>>(probe: &__private::Probe<E, T>, _: C) -> Self {
        Self {
            name: probe.name(),
            _p: PhantomData,
        }
    }
    pub fn name(&self) -> &'static str {
        self.name
    }
    pub fn kind(&self) -> ColumnKind {
        K::KIND
    }
    pub fn path(self, seg: &'static str) -> JsonPath
    where
        K: IsJson,
    {
        JsonPath {
            column: self.name,
            path: vec![seg],
        }
    }
}

/// `col!(Entity::field)` -> `Column<Entity, FieldTy, Kind>`; expression context only.
#[macro_export]
macro_rules! col {
    ($e:ident :: $f:ident) => {{
        #[allow(unused_imports)]
        use $crate::__private::{JsonLevel as _, NullableJsonLevel as _, ScalarLevel as _};
        let probe = <$e>::__rupa_fields().$f;
        $crate::Column::from_probe(&probe, (&&&probe).__rupa_codec())
    }};
}

#[doc(hidden)]
pub mod __private {
    use super::*;

    pub struct Probe<E, T> {
        name: &'static str,
        _p: PhantomData<fn() -> (E, T)>,
    }
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

    // Level 1 — receiver `&&&Probe`, matched first.
    pub trait ScalarLevel<T> {
        fn __rupa_codec(&self) -> ScalarCodec<T>;
    }
    impl<E, T: ScalarColumn> ScalarLevel<T> for &&Probe<E, T> {
        fn __rupa_codec(&self) -> ScalarCodec<T> {
            ScalarCodec::new()
        }
    }

    // Level 2 — after one auto-deref.
    pub trait NullableJsonLevel<T> {
        fn __rupa_codec(&self) -> NullableJsonCodec<T>;
    }
    impl<E, T: JsonColumn> NullableJsonLevel<T> for &Probe<E, Option<T>> {
        fn __rupa_codec(&self) -> NullableJsonCodec<T> {
            NullableJsonCodec(PhantomData)
        }
    }

    // Level 3 — fallback; bound on the method so errors name `JsonColumn`.
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

    pub fn kind_of<T, C: Codec<T>>(_: &C) -> ColumnKind {
        <C::Kind as KindMarker>::KIND
    }
    pub fn nullable_of<T, C: Codec<T>>(_: &C) -> bool {
        C::NULLABLE
    }
}
