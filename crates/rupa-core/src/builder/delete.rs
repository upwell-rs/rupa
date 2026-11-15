//! `DELETE`.

use std::marker::PhantomData;

use crate::capability::Deletable;
use crate::entity::{Entity, IdValues, Relation};
use crate::expr::IntoExpr;
use crate::ir::{Delete, Statement};
use crate::query::{AffectedResult, Expect, Query};

use super::{and_filter, key_filter};

/// Starts a delete from `E`, an entity or a `rupa::table!` relation.
/// Requires `E: Deletable<E>`.
pub fn delete<E: Relation + Deletable<E>>() -> DeleteBuilder<E> {
    DeleteBuilder {
        delete: Delete {
            table: E::TABLE,
            filter: None,
        },
        _e: PhantomData,
    }
}

#[derive(Debug, Clone)]
/// A delete being built.
pub struct DeleteBuilder<E> {
    delete: Delete,
    _e: PhantomData<fn() -> E>,
}

impl<E: Entity + Deletable<E>> DeleteBuilder<E> {
    /// Deletes the row with id `id`.
    pub fn by_id(mut self, id: &E::Id) -> Self {
        and_filter(
            &mut self.delete.filter,
            key_filter(E::ID_COLUMNS, id.id_values()),
        );
        self
    }
}

impl<E: Relation + Deletable<E>> DeleteBuilder<E> {
    /// Deletes the row `value` identifies.
    pub fn one<V: Deletable<E>>(mut self, value: &V) -> Self {
        and_filter(
            &mut self.delete.filter,
            key_filter(E::ID_COLUMNS, value.delete_key()),
        );
        self
    }

    /// Adds a condition; repeated calls are ANDed. Without one, every row is
    /// deleted.
    pub fn filter(mut self, cond: impl IntoExpr<bool>) -> Self {
        and_filter(&mut self.delete.filter, cond.into_node());
        self
    }

    /// Finishes as an affected-row result (`u64` or `bool`).
    pub fn build<R: AffectedResult>(self) -> Query<R> {
        Query::new(Statement::Delete(self.delete), Expect::Affected)
    }

    /// Finishes, returning the number of deleted rows.
    pub fn affected(self) -> Query<u64> {
        self.build()
    }
}
