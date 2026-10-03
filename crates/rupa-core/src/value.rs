//! Runtime values. A [`Value`] is the only way data enters a query: the IR
//! holds values only inside [`ExprNode::Param`](crate::ir::ExprNode::Param),
//! and renderers always emit them as bound parameters.

use crate::error::DecodeError;

/// SQL-level type tag. Used for typed nulls, casts and as a decode hint for drivers.
/// Deliberately exhaustive so every renderer and driver must map new variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SqlType {
    Bool,
    I16,
    I32,
    I64,
    F32,
    F64,
    Text,
    Bytes,
    Uuid,
    Date,
    Time,
    Timestamp,
    TimestampTz,
    Json,
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// A typed null: drivers need a type for otherwise unannotated parameters.
    Null(SqlType),
    Bool(bool),
    I16(i16),
    I32(i32),
    I64(i64),
    F32(f32),
    F64(f64),
    Text(String),
    Bytes(Vec<u8>),
    #[cfg(feature = "uuid")]
    Uuid(uuid::Uuid),
    #[cfg(feature = "chrono")]
    Date(chrono::NaiveDate),
    #[cfg(feature = "chrono")]
    Time(chrono::NaiveTime),
    #[cfg(feature = "chrono")]
    Timestamp(chrono::NaiveDateTime),
    #[cfg(feature = "chrono")]
    TimestampTz(chrono::DateTime<chrono::Utc>),
    Json(serde_json::Value),
}

impl Value {
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null(_))
    }
}

/// Marker + codec for types stored as a plain scalar column.
///
/// Field types implementing this are scalar columns; other
/// `Serialize + DeserializeOwned` types are inferred as JSON columns.
#[diagnostic::on_unimplemented(
    message = "`{Self}` is not a scalar column type",
    note = "implement `ScalarColumn` for it, or store it as JSON with `#[column(json)]`"
)]
pub trait ScalarColumn: Sized + 'static {
    /// The non-nullable type underneath: `i64` for both `i64` and `Option<i64>`.
    /// Comparisons accept values of the base type.
    type Base: ScalarColumn;
    const SQL_TYPE: SqlType;
    const NULLABLE: bool = false;
    fn to_value(&self) -> Value;
    fn from_value(v: Value) -> Result<Self, DecodeError>;
}

macro_rules! scalar {
    ($($(#[$m:meta])* $t:ty => $variant:ident),* $(,)?) => {$(
        $(#[$m])*
        impl ScalarColumn for $t {
            type Base = $t;
            const SQL_TYPE: SqlType = SqlType::$variant;
            fn to_value(&self) -> Value {
                Value::$variant(self.clone())
            }
            fn from_value(v: Value) -> Result<Self, DecodeError> {
                match v {
                    Value::$variant(x) => Ok(x),
                    other => Err(DecodeError::type_mismatch(SqlType::$variant, &other)),
                }
            }
        }
    )*};
}

scalar! {
    bool => Bool,
    i16 => I16,
    i32 => I32,
    i64 => I64,
    f32 => F32,
    f64 => F64,
    String => Text,
    Vec<u8> => Bytes,
    #[cfg(feature = "uuid")] uuid::Uuid => Uuid,
    #[cfg(feature = "chrono")] chrono::NaiveDate => Date,
    #[cfg(feature = "chrono")] chrono::NaiveTime => Time,
    #[cfg(feature = "chrono")] chrono::NaiveDateTime => Timestamp,
    #[cfg(feature = "chrono")] chrono::DateTime<chrono::Utc> => TimestampTz,
}

impl<T: ScalarColumn> ScalarColumn for Option<T> {
    type Base = T::Base;
    const SQL_TYPE: SqlType = T::SQL_TYPE;
    const NULLABLE: bool = true;
    fn to_value(&self) -> Value {
        match self {
            Some(v) => v.to_value(),
            None => Value::Null(T::SQL_TYPE),
        }
    }
    fn from_value(v: Value) -> Result<Self, DecodeError> {
        match v {
            Value::Null(_) => Ok(None),
            v => T::from_value(v).map(Some),
        }
    }
}
