//! `UPDATE`.

use std::marker::PhantomData;

use crate::capability::{RowKey, Updatable};
use crate::column::{Column, Scalar};
use crate::entity::Relation;
use crate::expr::IntoExpr;
use crate::ir::{ExprNode, Statement, Update};
use crate::query::{AffectedResult, Expect, Query};
use crate::value::ScalarColumn;

use super::{and_filter, key_filter};

/// Starts an update of `E` rows, an entity or a `rupa::table!` relation;
/// set columns with `one`, `apply`, `set` or `set_expr`, and restrict it
/// with `filter`.
pub fn update<E: Relation>() -> UpdateBuilder<E> {
    UpdateBuilder {
        update: Update {
            table: E::TABLE,
            set: Vec::new(),
            filter: None,
        },
        _e: PhantomData,
    }
}

#[derive(Debug, Clone)]
/// An update being built.
pub struct UpdateBuilder<E> {
    update: Update,
    _e: PhantomData<fn() -> E>,
}

impl<E: Relation> UpdateBuilder<E> {
    /// Updates the row `value` identifies: a full entity, or a patch with an id.
    pub fn one<V: Updatable<E>>(mut self, value: &V) -> Self
    where
        V::Key: RowKey,
    {
        self.push_values(value);
        and_filter(
            &mut self.update.filter,
            key_filter(E::ID_COLUMNS, value.key().key_values()),
        );
        self
    }

    /// Adds `value`'s assignments without restricting rows; combine with
    /// `filter`. A value with no assignments (a `table!` marker itself)
    /// leaves the update empty, which rendering refuses.
    pub fn apply<V: Updatable<E>>(mut self, value: &V) -> Self {
        self.push_values(value);
        self
    }

    fn push_values<V: Updatable<E>>(&mut self, value: &V) {
        self.update.set.extend(
            value
                .update_values()
                .into_iter()
                .map(|(c, v)| (c, ExprNode::Param(v))),
        );
    }

    /// Sets a column of `E` to a field value, encoded as the entity stores it
    /// (scalar or JSON). Requires `E: Updatable<E>`.
    pub fn set<T, K>(mut self, column: Column<E, T, K>, value: &T) -> Self
    where
        E: Updatable<E>,
    {
        self.update
            .set
            .push((column.name(), ExprNode::Param(column.encode(value))));
        self
    }

    /// Sets a scalar column of `E` to an expression, e.g. another column.
    /// Requires `E: Updatable<E>`.
    pub fn set_expr<T: ScalarColumn>(
        mut self,
        column: Column<E, T, Scalar>,
        expr: impl IntoExpr<T::Base>,
    ) -> Self
    where
        E: Updatable<E>,
    {
        self.update.set.push((column.name(), expr.into_node()));
        self
    }

    /// Adds a condition; repeated calls are ANDed. Without one, every row is
    /// updated.
    pub fn filter(mut self, cond: impl IntoExpr<bool>) -> Self {
        and_filter(&mut self.update.filter, cond.into_node());
        self
    }

    /// Finishes as an affected-row result (`u64` or `bool`).
    pub fn build<R: AffectedResult>(self) -> Query<R> {
        Query::new(Statement::Update(self.update), Expect::Affected)
    }

    /// Finishes, returning the number of matched rows.
    pub fn affected(self) -> Query<u64> {
        self.build()
    }
}
