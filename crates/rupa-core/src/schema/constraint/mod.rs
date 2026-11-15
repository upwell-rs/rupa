//! Table constraints: named `CHECK` and multi-column `UNIQUE`.
//!
//! A [`Check`] holds a query expression, a [`Unique`] typed column handles,
//! the same way indexes do. A table carries them in
//! [`Table::constraints`](super::Table::constraints), and an existing table
//! gains or loses one through
//! [`SchemaOp::AddConstraint`](super::SchemaOp::AddConstraint) and
//! [`SchemaOp::DropConstraint`](super::SchemaOp::DropConstraint). Dialects
//! that cannot express one refuse it when the plan is rendered.

use std::marker::PhantomData;

use super::{DdlExpr, IndexColumn};
use crate::column::{Column, Dynamic};
use crate::entity::Relation;
use crate::expr::IntoCondition;
use crate::ir::TableRef;

/// A named `CHECK (<condition>)` constraint.
///
/// MySQL enforces `CHECK` from 8.0.16 on; earlier servers parse it and
/// silently ignore it. rupa cannot tell the server version, so a plan that
/// relies on a check must target MySQL 8.0.16 or newer.
///
/// MySQL also refuses a check over an `AUTO_INCREMENT` column or over a column
/// of a foreign key whose delete action is `CASCADE` or `SET NULL`. Rendering
/// a `CREATE TABLE` refuses those, since the table's columns are known; a
/// check added to an existing table is left to the database.
#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    /// The constraint name.
    pub name: &'static str,
    /// The condition every row must meet.
    pub condition: DdlExpr,
}

impl Check {
    /// A check over an expression written without an entity, unchecked:
    /// prefer [`Check::on`].
    pub const fn new(name: &'static str, condition: DdlExpr) -> Self {
        Self { name, condition }
    }

    /// A check over `condition`, which may name the columns of entity `E`
    /// only: rendering refuses a column of another table.
    pub fn on<E: Relation>(name: &'static str, condition: impl IntoCondition) -> Self {
        Self::new(name, DdlExpr::of::<E, bool>(condition.into_condition()))
    }
}

/// A named, possibly multi-column `UNIQUE` constraint. Columns carry their
/// types: MySQL refuses unbounded `Text`, `Bytes` and `Json` in a key, which
/// only rendering can tell. A constraint built from typed handles records
/// their entity's table, and rendering refuses it on any other table.
#[derive(Debug, Clone, PartialEq)]
pub struct Unique {
    /// The constraint name.
    pub name: &'static str,
    /// The table the typed handles belong to, if known. It is checked
    /// against the constrained table, and is not part of the checksum.
    pub table: Option<TableRef>,
    /// The constrained columns, in order.
    pub columns: Vec<IndexColumn>,
}

impl Unique {
    /// A constraint over `columns`, for tables written as literals. Typed
    /// handles check more: see [`Unique::on`].
    pub fn new(name: &'static str, columns: impl IntoIterator<Item = IndexColumn>) -> Self {
        Self {
            name,
            table: None,
            columns: columns.into_iter().collect(),
        }
    }

    /// A constraint over one column of entity `E`; add more with
    /// [`UniqueBuilder::and`].
    pub fn on<E: Relation, T, K>(name: &'static str, column: Column<E, T, K>) -> UniqueBuilder<E> {
        UniqueBuilder {
            unique: Self {
                table: Some(E::TABLE),
                ..Self::new(name, [IndexColumn::of(column)])
            },
            _p: PhantomData,
        }
    }

    /// A constraint over one column of a table named only by the handle
    /// ([`Column::dynamic`]); add more with [`UniqueBuilder::and`]. Rendering
    /// refuses it on any other table.
    pub fn on_dynamic<T, K>(
        name: &'static str,
        column: Column<Dynamic, T, K>,
    ) -> UniqueBuilder<Dynamic> {
        UniqueBuilder {
            unique: Self {
                table: Some(column.declared_table()),
                ..Self::new(name, [IndexColumn::of(column)])
            },
            _p: PhantomData,
        }
    }
}

/// A [`Unique`] over columns of relation `E`, or of one named table for
/// [`Dynamic`]. Becomes a [`Unique`] or a [`Constraint`] where one is
/// expected.
#[derive(Debug, Clone, PartialEq)]
pub struct UniqueBuilder<E> {
    unique: Unique,
    _p: PhantomData<fn() -> E>,
}

impl<E: Relation> UniqueBuilder<E> {
    /// Adds a column of the entity.
    pub fn and<T, K>(mut self, column: Column<E, T, K>) -> Self {
        self.unique.columns.push(IndexColumn::of(column));

        self
    }
}

impl<E> UniqueBuilder<E> {
    /// The finished constraint.
    pub fn build(self) -> Unique {
        self.unique
    }
}

impl UniqueBuilder<Dynamic> {
    /// Adds a column of the same table.
    ///
    /// # Panics
    /// If the column belongs to another table.
    pub fn and<T, K>(mut self, column: Column<Dynamic, T, K>) -> Self {
        assert_eq!(
            Some(column.declared_table()),
            self.unique.table,
            "a unique constraint covers columns of one table"
        );

        self.unique.columns.push(IndexColumn::of(column));

        self
    }
}

impl<E> From<UniqueBuilder<E>> for Unique {
    fn from(builder: UniqueBuilder<E>) -> Self {
        builder.unique
    }
}

/// The kind of a [`Constraint`]. MySQL drops each kind with its own
/// statement, so dropping one names the kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ConstraintKind {
    /// A [`Check`].
    Check,
    /// A [`Unique`].
    Unique,
}

/// A table constraint. Deliberately exhaustive: a new kind must break every
/// renderer's and every checksum encoder's match.
#[derive(Debug, Clone, PartialEq)]
pub enum Constraint {
    /// A `CHECK` constraint.
    Check(Check),
    /// A `UNIQUE` constraint.
    Unique(Unique),
}

impl Constraint {
    /// The constraint name.
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Check(check) => check.name,
            Self::Unique(unique) => unique.name,
        }
    }

    /// The constraint's kind.
    pub const fn kind(&self) -> ConstraintKind {
        match self {
            Self::Check(_) => ConstraintKind::Check,
            Self::Unique(_) => ConstraintKind::Unique,
        }
    }
}

impl From<Check> for Constraint {
    fn from(check: Check) -> Self {
        Self::Check(check)
    }
}

impl From<Unique> for Constraint {
    fn from(unique: Unique) -> Self {
        Self::Unique(unique)
    }
}

impl<E> From<UniqueBuilder<E>> for Constraint {
    fn from(builder: UniqueBuilder<E>) -> Self {
        Self::Unique(builder.into())
    }
}

#[cfg(test)]
mod tests;
