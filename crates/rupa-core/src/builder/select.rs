//! `SELECT` and get-by-id.

use std::marker::PhantomData;

use crate::capability::Gettable;
use crate::entity::{Entity, IdValues};
use crate::expr::IntoExpr;
use crate::ir::{ColumnRef, FromClause, OrderBy, Select, Statement};
use crate::query::{Expect, Query, RowsResult};

use super::{and_filter, count_param, key_filter};

/// Starts a select of `E` rows (every column, in `E::COLUMNS` order).
pub fn select<E: Entity>() -> SelectBuilder<E> {
    SelectBuilder {
        select: Select {
            from: FromClause::Table(E::TABLE),
            projection: E::COLUMNS.iter().map(|c| ColumnRef::new(c.name)).collect(),
            filter: None,
            order_by: Vec::new(),
            limit: None,
            offset: None,
        },
        _e: PhantomData,
    }
}

#[derive(Debug, Clone)]
/// A select being built; finish with [`all`](Self::all),
/// [`optional`](Self::optional), [`one`](Self::one) or `exists`.
pub struct SelectBuilder<E> {
    pub(crate) select: Select,
    _e: PhantomData<fn() -> E>,
}

impl<E: Entity> SelectBuilder<E> {
    /// Adds a condition; repeated calls are ANDed.
    pub fn filter(mut self, cond: impl IntoExpr<bool>) -> Self {
        and_filter(&mut self.select.filter, cond.into_node());
        self
    }

    /// Adds a sort key; repeated calls sort by each in turn.
    pub fn order_by(mut self, order: OrderBy) -> Self {
        self.select.order_by.push(order);
        self
    }

    /// Returns at most `n` rows.
    pub fn limit(mut self, n: u64) -> Self {
        self.select.limit = Some(count_param(n));
        self
    }

    /// Skips the first `n` rows.
    pub fn offset(mut self, n: u64) -> Self {
        self.select.offset = Some(count_param(n));
        self
    }

    /// Builds with the result shape taken from `R`; used by generated code,
    /// which reads `R` off the method's return type.
    pub fn build<R: RowsResult<Item = E>>(self) -> Query<R> {
        Query::new(Statement::Select(self.select), R::EXPECT)
    }

    /// Every matching row.
    pub fn all(self) -> Query<Vec<E>> {
        self.build()
    }

    /// At most one row; more is [`ResultError::TooManyRows`](crate::ResultError::TooManyRows).
    pub fn optional(self) -> Query<Option<E>> {
        self.build()
    }

    /// Exactly one row; none is
    /// [`ResultError::NotFound`](crate::ResultError::NotFound), more is
    /// [`ResultError::TooManyRows`](crate::ResultError::TooManyRows).
    pub fn one(self) -> Query<E> {
        self.build()
    }

    /// Whether any row matches. Ordering, limit and offset are discarded.
    pub fn exists(mut self) -> Query<bool> {
        self.select.order_by.clear();
        self.select.limit = None;
        self.select.offset = None;
        Query::new(Statement::Exists(self.select), Expect::Exists)
    }
}

/// The row of `E` with the given id, if any. Requires `E: Gettable<E>`.
pub fn get<E: Entity + Gettable<E>>(id: &E::Id) -> Query<Option<E>> {
    let mut builder = select::<E>();
    and_filter(
        &mut builder.select.filter,
        key_filter(E::ID_COLUMNS, id.id_values()),
    );
    builder.optional()
}
