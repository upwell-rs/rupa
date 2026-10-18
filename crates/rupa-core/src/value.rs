//! Runtime values. A [`Value`] is the only way data enters a query: the IR
//! holds values only inside [`ExprNode::Param`](crate::ir::ExprNode::Param),
//! and renderers always emit them as bound parameters.

use crate::error::DecodeError;

/// SQL-level type tag. Used for typed nulls, casts and as a decode hint for drivers.
/// Deliberately exhaustive so every renderer and driver must map new variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SqlType {
    /// `boolean`.
    Bool,
    /// `smallint`.
    I16,
    /// `integer`.
    I32,
    /// `bigint`.
    I64,
    /// `real`.
    F32,
    /// `double precision`.
    F64,
    /// `text`.
    Text,
    /// `bytea` / `BLOB`.
    Bytes,
    /// `uuid` (text in dialects without one).
    Uuid,
    /// `date`.
    Date,
    /// `time` (without time zone).
    Time,
    /// `timestamp` (without time zone).
    Timestamp,
    /// `timestamptz`: an instant, stored in UTC.
    TimestampTz,
    /// `jsonb` / `JSON` / JSON text.
    Json,
}

impl SqlType {
    /// The cargo feature values of this type need (`"chrono"`, `"uuid"`), if
    /// any.
    pub const fn feature(self) -> Option<&'static str> {
        match self {
            SqlType::Uuid => Some("uuid"),
            SqlType::Date | SqlType::Time | SqlType::Timestamp | SqlType::TimestampTz => {
                Some("chrono")
            }
            _ => None,
        }
    }
}

#[non_exhaustive]
#[derive(Debug, Clone, PartialEq)]
/// A runtime value: a bound parameter, or a decoded column.
pub enum Value {
    /// A typed null: drivers need a type for otherwise unannotated parameters.
    Null(SqlType),
    /// A boolean.
    Bool(bool),
    /// A 16-bit integer.
    I16(i16),
    /// A 32-bit integer.
    I32(i32),
    /// A 64-bit integer.
    I64(i64),
    /// A 32-bit float.
    F32(f32),
    /// A 64-bit float.
    F64(f64),
    /// Text.
    Text(String),
    /// Bytes.
    Bytes(Vec<u8>),
    #[cfg(feature = "uuid")]
    /// A UUID.
    Uuid(uuid::Uuid),
    #[cfg(feature = "chrono")]
    /// A calendar date.
    Date(chrono::NaiveDate),
    #[cfg(feature = "chrono")]
    /// A time of day.
    Time(chrono::NaiveTime),
    #[cfg(feature = "chrono")]
    /// A date and time without a time zone.
    Timestamp(chrono::NaiveDateTime),
    #[cfg(feature = "chrono")]
    /// An instant in UTC.
    TimestampTz(chrono::DateTime<chrono::Utc>),
    /// A JSON document.
    Json(serde_json::Value),
}

impl Value {
    /// Whether this is a (typed) `NULL`.
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
    /// The SQL type values are bound and decoded as.
    const SQL_TYPE: SqlType;
    /// Whether the type admits `NULL` (`Option<T>`).
    const NULLABLE: bool = false;
    /// The value to bind.
    fn to_value(&self) -> Value;
    /// Decodes a column value.
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
