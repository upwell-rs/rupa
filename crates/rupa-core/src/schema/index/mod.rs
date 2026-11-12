//! Index definitions: a kind, typed columns and uniqueness.
//!
//! Kinds are designed around PostgreSQL. A dialect that cannot build a kind
//! refuses it when the plan is rendered, before any target is acquired;
//! which columns a kind accepts is checked by the compiler
//! ([`IndexableBy`]).

mod kinds;

use std::marker::PhantomData;

use super::{ColumnDef, ColumnType};
use crate::column::Column;
use crate::dialect::DialectId;
use crate::entity::Entity;
use crate::ir::TableRef;

pub use kinds::{BTree, Gin, Gist, Hash};

/// How a dialect writes an index kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IndexSupport {
    /// The dialect's default index method: nothing is written.
    Default,
    /// An explicit `USING <method>` clause.
    Using(&'static str),
    /// The dialect cannot build the kind, for the given reason.
    Unsupported(&'static str),
}

/// The data behind an index kind: its name and, per dialect, how it is
/// written and what it allows. Plain data, so an [`Index`] can be compared
/// and checksummed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IndexMethod {
    /// The kind's stable name.
    pub name: &'static str,
    /// Support on PostgreSQL.
    pub postgres: IndexSupport,
    /// Support on MySQL.
    pub mysql: IndexSupport,
    /// Support on SQLite.
    pub sqlite: IndexSupport,
    /// Whether the kind can enforce uniqueness.
    pub unique: bool,
    /// Whether the kind can cover several columns.
    pub multi_column: bool,
}

impl IndexMethod {
    /// A kind no dialect supports yet, covering several columns but not
    /// enforcing uniqueness.
    pub const fn new(name: &'static str) -> Self {
        const NONE: IndexSupport = IndexSupport::Unsupported("no dialect support is declared");

        Self {
            name,
            postgres: NONE,
            mysql: NONE,
            sqlite: NONE,
            unique: false,
            multi_column: true,
        }
    }

    /// Sets how PostgreSQL writes the kind.
    pub const fn postgres(mut self, support: IndexSupport) -> Self {
        self.postgres = support;

        self
    }

    /// Sets how MySQL writes the kind.
    pub const fn mysql(mut self, support: IndexSupport) -> Self {
        self.mysql = support;

        self
    }

    /// Sets how SQLite writes the kind.
    pub const fn sqlite(mut self, support: IndexSupport) -> Self {
        self.sqlite = support;

        self
    }

    /// The kind can enforce uniqueness.
    pub const fn supports_unique(mut self) -> Self {
        self.unique = true;

        self
    }

    /// The kind covers exactly one column.
    pub const fn single_column(mut self) -> Self {
        self.multi_column = false;

        self
    }

    /// How `dialect` writes the kind. A dialect without index DDL refuses it.
    pub const fn support(&self, dialect: DialectId) -> IndexSupport {
        match dialect {
            DialectId::Postgres => self.postgres,
            DialectId::MySql => self.mysql,
            DialectId::Sqlite => self.sqlite,
            _ => IndexSupport::Unsupported("the dialect has no index DDL"),
        }
    }
}

/// An index kind: a type that names an access method (`BTree`, `Gin`, or
/// your own) and, through [`IndexableBy`], the column types it accepts.
///
/// ```
/// use rupa_core::schema::{IndexKind, IndexMethod, IndexSupport, IndexableBy};
///
/// struct Brin;
///
/// impl IndexKind for Brin {
///     const METHOD: IndexMethod =
///         IndexMethod::new("brin").postgres(IndexSupport::Using("brin"));
/// }
///
/// impl IndexableBy<Brin> for i64 {}
/// ```
pub trait IndexKind: Sized + 'static {
    /// The kind's data.
    const METHOD: IndexMethod;

    /// An index of this kind over one column of entity `E`, which must hold
    /// a type the kind accepts.
    fn on<E: Entity, T: IndexableBy<Self>, C>(
        name: &'static str,
        column: Column<E, T, C>,
    ) -> IndexBuilder<Self, E> {
        IndexBuilder {
            index: Index::new(name, E::TABLE, [IndexColumn::of(column)]).method(Self::METHOD),
            _p: PhantomData,
        }
    }
}

/// Marks a column type `Self` as indexable by kind `K`: `GIN` over an
/// integer, for one, does not compile.
#[diagnostic::on_unimplemented(
    message = "a `{Self}` column cannot be indexed by `{K}`",
    label = "not accepted by this index kind",
    note = "implement `IndexableBy<{K}>` for the type if the kind can index it"
)]
pub trait IndexableBy<K: IndexKind> {}

/// An indexed column: its name and type, so rendering can check the type
/// against the dialect.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IndexColumn {
    /// The column name.
    pub name: &'static str,
    /// The column type, with its length for bounded text.
    pub ty: ColumnType,
}

impl IndexColumn {
    /// A column named `name` of type `ty`.
    pub const fn new(name: &'static str, ty: ColumnType) -> Self {
        Self { name, ty }
    }

    /// The column an entity field handle describes.
    pub const fn of<E, T, K>(column: Column<E, T, K>) -> Self {
        Self::new(column.name(), column.column_type())
    }
}

impl From<&ColumnDef> for IndexColumn {
    fn from(column: &ColumnDef) -> Self {
        Self::new(column.name, column.ty)
    }
}

/// `CREATE [UNIQUE] INDEX`. Columns carry their types: MySQL refuses unbounded
/// `Text`, `Bytes` and `Json` in an index, which only rendering can tell.
#[derive(Debug, Clone, PartialEq)]
pub struct Index {
    /// The index name.
    pub name: &'static str,
    /// The indexed table.
    pub table: TableRef,
    /// The index kind.
    pub kind: IndexMethod,
    /// The indexed columns, in order.
    pub columns: Vec<IndexColumn>,
    /// Whether indexed values must be unique.
    pub unique: bool,
}

impl Index {
    /// A B-tree index named `name` over `columns` of `table`, for tables
    /// written as literals. Typed handles check more: see [`IndexKind::on`].
    pub fn new(
        name: &'static str,
        table: TableRef,
        columns: impl IntoIterator<Item = IndexColumn>,
    ) -> Self {
        Self {
            name,
            table,
            kind: BTree::METHOD,
            columns: columns.into_iter().collect(),
            unique: false,
        }
    }

    /// A B-tree index named `name` over one entity column.
    pub fn on<E: Entity, T: IndexableBy<BTree>, C>(
        name: &'static str,
        column: Column<E, T, C>,
    ) -> Self {
        BTree::on(name, column).into()
    }

    /// Sets the index kind, unchecked: prefer [`IndexKind::on`].
    pub fn method(mut self, kind: IndexMethod) -> Self {
        self.kind = kind;

        self
    }

    /// Makes indexed values unique.
    pub fn unique(mut self) -> Self {
        self.unique = true;

        self
    }
}

/// An index of kind `K` over columns of entity `E`, each checked against the
/// kind. Becomes an [`Index`] where one is expected.
#[derive(Debug, Clone, PartialEq)]
pub struct IndexBuilder<K, E> {
    index: Index,
    _p: PhantomData<fn() -> (K, E)>,
}

impl<K: IndexKind, E: Entity> IndexBuilder<K, E> {
    /// Adds a column the kind accepts.
    pub fn and<T: IndexableBy<K>, C>(mut self, column: Column<E, T, C>) -> Self {
        self.index.columns.push(IndexColumn::of(column));

        self
    }

    /// Makes indexed values unique.
    pub fn unique(mut self) -> Self {
        self.index.unique = true;

        self
    }

    /// The finished index.
    pub fn build(self) -> Index {
        self.index
    }
}

impl<K, E> From<IndexBuilder<K, E>> for Index {
    fn from(builder: IndexBuilder<K, E>) -> Self {
        builder.index
    }
}

#[cfg(test)]
mod tests;
