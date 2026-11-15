//! `INSERT`.

use std::marker::PhantomData;

use crate::capability::Insertable;
use crate::entity::{Entity, Relation};
use crate::ir::{ColumnRef, ExprNode, Insert, Statement};
use crate::query::{AffectedResult, Expect, Query, RowsResult};

/// Starts an insert into `E`. Rows are added with values implementing
/// [`Insertable<E>`]: the entity itself, or a dedicated input struct.
pub fn insert<E: Relation>() -> InsertBuilder<E> {
    InsertBuilder { _e: PhantomData }
}

#[derive(Debug, Clone, Copy)]
/// An insert into `E` without rows yet; add them with
/// [`value`](Self::value) or [`values`](Self::values).
pub struct InsertBuilder<E> {
    _e: PhantomData<fn() -> E>,
}

impl<E: Relation> InsertBuilder<E> {
    /// Inserts `value`.
    pub fn value<V: Insertable<E>>(self, value: &V) -> InsertRows<E, V> {
        InsertRows::new().value(value)
    }

    /// Inserts every value, as one multi-row statement.
    pub fn values<'a, V: Insertable<E> + 'a>(
        self,
        values: impl IntoIterator<Item = &'a V>,
    ) -> InsertRows<E, V> {
        InsertRows::new().values(values)
    }
}

/// An insert of one or more values of the same type `V`, so every row has
/// the same columns.
#[derive(Debug, Clone)]
pub struct InsertRows<E, V> {
    insert: Insert,
    _p: PhantomData<fn() -> (E, V)>,
}

impl<E: Relation, V: Insertable<E>> InsertRows<E, V> {
    fn new() -> Self {
        Self {
            insert: Insert {
                table: E::TABLE,
                columns: Vec::new(),
                rows: Vec::new(),
                returning: None,
            },
            _p: PhantomData,
        }
    }

    /// Adds a row.
    pub fn value(mut self, value: &V) -> Self {
        let pairs = value.insert_values();
        if self.insert.rows.is_empty() {
            self.insert.columns = pairs.iter().map(|(c, _)| *c).collect();
        }

        debug_assert!(
            self.insert
                .columns
                .iter()
                .copied()
                .eq(pairs.iter().map(|(c, _)| *c)),
            "Insertable impls must yield the same columns for every value"
        );
        self.insert
            .rows
            .push(pairs.into_iter().map(|(_, v)| ExprNode::Param(v)).collect());
        self
    }

    /// Adds rows.
    pub fn values<'a>(self, values: impl IntoIterator<Item = &'a V>) -> Self
    where
        V: 'a,
    {
        values.into_iter().fold(self, Self::value)
    }

    /// Finishes as an affected-row result (`u64` or `bool`).
    pub fn build<R: AffectedResult>(self) -> Query<R> {
        Query::new(Statement::Insert(self.insert), Expect::Affected)
    }

    /// Finishes, returning the number of inserted rows.
    pub fn affected(self) -> Query<u64> {
        self.build()
    }

    /// Returns the inserted rows as `E`, including database-generated values.
    /// Requires a dialect with `RETURNING`; others fail when rendered.
    pub fn returning<R: RowsResult<Item = E>>(mut self) -> Query<R> {
        self.insert.returning = Some(E::COLUMNS.iter().map(|c| ColumnRef::new(c.name)).collect());
        Query::new(Statement::Insert(self.insert), R::EXPECT)
    }
}

impl<E: Entity, V: Insertable<E>> InsertRows<E, V> {
    /// Returns the one inserted row (see [`returning`](Self::returning)).
    pub fn returning_one(self) -> Query<E> {
        self.returning()
    }

    /// Returns every inserted row (see [`returning`](Self::returning)).
    pub fn returning_all(self) -> Query<Vec<E>> {
        self.returning()
    }
}
