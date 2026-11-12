//! Extensible column types: a kind names a SQL type per dialect.
//!
//! A kind is plain const data, so a [`ColumnType::Kind`] can be compared and
//! checksummed. A dialect that cannot store a kind refuses it when the plan
//! is rendered, before any target is acquired.

use super::ColumnType;
use crate::dialect::DialectId;
use crate::value::SqlType;

/// How a dialect writes a column type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TypeSupport {
    /// The type `name`, written `name(arg, ..)` or bare `name` without
    /// arguments.
    Named {
        /// The type's name.
        name: &'static str,
        /// Numeric arguments, in order.
        args: &'static [u32],
    },
    /// The dialect cannot store the type, for the given reason.
    Unsupported(&'static str),
}

impl TypeSupport {
    /// The type `name`, without arguments.
    pub const fn named(name: &'static str) -> Self {
        Self::Named { name, args: &[] }
    }

    /// The type `name` with numeric `args`, as in `vector(3)`.
    pub const fn with_args(name: &'static str, args: &'static [u32]) -> Self {
        Self::Named { name, args }
    }
}

/// The data behind a column kind: its name and, per dialect, the type it is
/// stored as. Plain data, so a [`ColumnType`] holding it can be compared and
/// checksummed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColumnSpec {
    /// The kind's stable name.
    pub name: &'static str,
    /// Support on PostgreSQL.
    pub postgres: TypeSupport,
    /// Support on MySQL.
    pub mysql: TypeSupport,
    /// Support on SQLite.
    pub sqlite: TypeSupport,
    /// The SQL type drivers bind and decode values as.
    pub sql_type: SqlType,
    /// Whether MySQL can key and index the type without a prefix length.
    pub bounded: bool,
}

impl ColumnSpec {
    /// A kind no dialect supports yet, bound and decoded as `sql_type` and
    /// not usable in a MySQL key or index.
    pub const fn new(name: &'static str, sql_type: SqlType) -> Self {
        const NONE: TypeSupport = TypeSupport::Unsupported("no dialect support is declared");

        Self {
            name,
            postgres: NONE,
            mysql: NONE,
            sqlite: NONE,
            sql_type,
            bounded: false,
        }
    }

    /// Sets how PostgreSQL writes the type.
    pub const fn postgres(mut self, support: TypeSupport) -> Self {
        self.postgres = support;

        self
    }

    /// Sets how MySQL writes the type.
    pub const fn mysql(mut self, support: TypeSupport) -> Self {
        self.mysql = support;

        self
    }

    /// Sets how SQLite writes the type.
    pub const fn sqlite(mut self, support: TypeSupport) -> Self {
        self.sqlite = support;

        self
    }

    /// The type has a bounded size, so MySQL can key and index it.
    pub const fn bounded(mut self) -> Self {
        self.bounded = true;

        self
    }

    /// How `dialect` writes the type. A dialect without column DDL refuses it.
    pub const fn support(&self, dialect: DialectId) -> TypeSupport {
        match dialect {
            DialectId::Postgres => self.postgres,
            DialectId::MySql => self.mysql,
            DialectId::Sqlite => self.sqlite,
            _ => TypeSupport::Unsupported("the dialect has no column DDL"),
        }
    }

    /// The column type of a column of this kind. A reference keeps
    /// [`ColumnType`] small, as every column definition and error carries one.
    pub const fn column_type(&'static self) -> ColumnType {
        ColumnType::Kind(self)
    }
}

/// A column kind: a Rust value type that names the SQL type it is stored as
/// on each dialect, even when the type is parameterized by const generics.
///
/// A kind is always supported in DDL (`CREATE TABLE`, `ADD COLUMN`) and in
/// indexes, where [`IndexableBy`](super::IndexableBy) opts it into index
/// kinds. Its values round-trip through entities only when every driver can
/// bind the spec's [`sql_type`](ColumnSpec::sql_type) directly into the
/// native type and decode it back: `BigText` below is stored as `TEXT`,
/// `MEDIUMTEXT` and `CLOB`, so a text codec works. A kind whose native type
/// needs a cast (PostgreSQL `tsvector`, `vector(N)`) is not an entity field
/// type yet: write and read its values with raw SQL until
/// [#91](https://github.com/upwell-rs/rupa/issues/91) adds casts to the
/// query IR.
///
/// An entity field type implements the scalar codec next to the kind, whose
/// `COLUMN_TYPE` points back at the spec.
///
/// ```
/// use rupa_core::schema::{ColumnKind, ColumnSpec, ColumnType, TypeSupport};
/// use rupa_core::{DecodeError, ScalarColumn, SqlType, Value};
///
/// struct BigText(String);
///
/// impl ColumnKind for BigText {
///     const SPEC: ColumnSpec = ColumnSpec::new("big-text", SqlType::Text)
///         .postgres(TypeSupport::named("TEXT"))
///         .mysql(TypeSupport::named("MEDIUMTEXT"))
///         .sqlite(TypeSupport::named("CLOB"));
/// }
///
/// impl ScalarColumn for BigText {
///     type Base = Self;
///     const SQL_TYPE: SqlType = Self::SPEC.sql_type;
///     const COLUMN_TYPE: ColumnType = Self::SPEC.column_type();
///
///     fn to_value(&self) -> Value {
///         Value::Text(self.0.clone())
///     }
///
///     fn from_value(v: Value) -> Result<Self, DecodeError> {
///         match v {
///             Value::Text(text) => Ok(Self(text)),
///             other => Err(DecodeError::type_mismatch(SqlType::Text, &other)),
///         }
///     }
/// }
///
/// assert_eq!(BigText::SPEC.column_type(), <BigText as ScalarColumn>::COLUMN_TYPE);
/// ```
///
/// A kind that needs a cast is declared without the codec, so it has no
/// entity column handle. Its columns and indexes are written literally, and a
/// literal index is not checked against [`IndexableBy`](super::IndexableBy):
///
/// ```
/// use rupa_core::ir::TableRef;
/// use rupa_core::schema::{
///     ColumnDef, ColumnKind, ColumnSpec, Gin, Index, IndexColumn, IndexKind, TypeSupport,
/// };
/// use rupa_core::SqlType;
///
/// struct Tsvector;
///
/// impl ColumnKind for Tsvector {
///     const SPEC: ColumnSpec = ColumnSpec::new("tsvector", SqlType::Text)
///         .postgres(TypeSupport::named("TSVECTOR"))
///         .mysql(TypeSupport::Unsupported("MySQL has no tsvector type"))
///         .sqlite(TypeSupport::Unsupported("SQLite has no tsvector type"));
/// }
///
/// let terms = ColumnDef::new("terms", Tsvector::SPEC.column_type());
/// let index = Index::new(
///     "docs_terms",
///     TableRef::new("docs"),
///     [IndexColumn::new("terms", Tsvector::SPEC.column_type())],
/// )
/// .method(Gin::METHOD);
///
/// assert_eq!(terms.name, "terms");
/// assert_eq!(index.kind, Gin::METHOD);
/// ```
///
/// Const generics become structured arguments:
///
/// ```
/// use rupa_core::schema::{ColumnKind, ColumnSpec, TypeSupport};
/// use rupa_core::SqlType;
///
/// struct Vector<const N: usize>([f32; N]);
///
/// impl<const N: usize> ColumnKind for Vector<N> {
///     const SPEC: ColumnSpec = ColumnSpec::new("vector", SqlType::Text)
///         .postgres(TypeSupport::with_args("vector", &[N as u32]))
///         .mysql(TypeSupport::Unsupported("MySQL has no vector type"))
///         .sqlite(TypeSupport::Unsupported("SQLite has no vector type"));
/// }
///
/// assert_ne!(Vector::<3>::SPEC, Vector::<4>::SPEC);
/// ```
pub trait ColumnKind: Sized + 'static {
    /// The kind's data.
    const SPEC: ColumnSpec;
}

#[cfg(test)]
mod tests;
