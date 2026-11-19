//! Bounded text that keeps its length in its type.

use std::borrow::Borrow;
use std::fmt;
use std::ops::Deref;

use serde::de::{Deserialize, Deserializer, Error as _};
use serde::{Serialize, Serializer};

use crate::error::DecodeError;
use crate::schema::ColumnType;
use crate::value::{ScalarColumn, SqlType, Value};

/// Text of at most `N` characters, stored as `VARCHAR(N)`.
///
/// As an entity field type it gives the column a length, so MySQL can index
/// it through portable plans where unbounded `String` columns are refused.
/// Construction checks the length; a stored value longer than `N` fails to
/// decode.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VarChar<const N: usize>(String);

/// A string longer than the `VarChar` it was meant for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("text of {len} characters exceeds the limit of {max}")]
pub struct TooLong {
    /// The limit.
    pub max: usize,
    /// The text's length in characters.
    pub len: usize,
}

impl<const N: usize> VarChar<N> {
    const LENGTH: u32 = {
        assert!(
            N > 0 && N <= u32::MAX as usize,
            "a VarChar length must be between 1 and u32::MAX"
        );

        N as u32
    };

    /// `text`, if it has at most `N` characters.
    pub fn new(text: impl Into<String>) -> Result<Self, TooLong> {
        let text = text.into();
        let len = text.chars().count();

        if len > N {
            return Err(TooLong { max: N, len });
        }

        Ok(Self(text))
    }

    /// The text.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The text, without the length bound.
    pub fn into_string(self) -> String {
        self.0
    }
}

impl<const N: usize> Deref for VarChar<N> {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl<const N: usize> AsRef<str> for VarChar<N> {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl<const N: usize> Borrow<str> for VarChar<N> {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl<const N: usize> fmt::Display for VarChar<N> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<const N: usize> TryFrom<String> for VarChar<N> {
    type Error = TooLong;

    fn try_from(text: String) -> Result<Self, TooLong> {
        Self::new(text)
    }
}

impl<const N: usize> TryFrom<&str> for VarChar<N> {
    type Error = TooLong;

    fn try_from(text: &str) -> Result<Self, TooLong> {
        Self::new(text)
    }
}

impl<const N: usize> Serialize for VarChar<N> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

impl<'de, const N: usize> Deserialize<'de> for VarChar<N> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;

        Self::new(text).map_err(D::Error::custom)
    }
}

impl<const N: usize> ScalarColumn for VarChar<N> {
    type Base = Self;
    const SQL_TYPE: SqlType = SqlType::Text;
    const COLUMN_TYPE: ColumnType = ColumnType::VarChar(Self::LENGTH);

    fn to_value(&self) -> Value {
        Value::Text(self.0.clone())
    }

    fn from_value(v: Value) -> Result<Self, DecodeError> {
        match v {
            Value::Text(text) => Self::new(text).map_err(|e| DecodeError::new(e.to_string())),
            other => Err(DecodeError::type_mismatch(SqlType::Text, &other)),
        }
    }
}

#[cfg(test)]
mod tests;
