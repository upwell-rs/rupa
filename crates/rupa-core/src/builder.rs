//! Typed statement builders. Each produces a dialect-free [`Query<R>`]; no IO.

use std::marker::PhantomData;

use crate::capability::{Deletable, Gettable, Insertable, RowKey, Updatable};
use crate::column::{Column, Scalar};
use crate::entity::{Entity, IdValues};
use crate::expr::IntoExpr;
use crate::ir::{
    BinOp, ColumnRef, Delete, ExprNode, FromClause, Insert, OrderBy, RawPart, RawSql, Select,
    Statement, Update,
};
use crate::query::{AffectedResult, Expect, Query, RowsResult};
use crate::value::{ScalarColumn, Value};

pub(crate) fn and_filter(filter: &mut Option<ExprNode>, cond: ExprNode) {
    *filter = Some(match filter.take() {
        None => cond,
        Some(prev) => ExprNode::Binary(BinOp::And, Box::new(prev), Box::new(cond)),
    });
}

/// SQL LIMIT/OFFSET are signed 64-bit; anything larger means "no bound".
pub(crate) fn count_param(n: u64) -> ExprNode {
    ExprNode::Param(Value::I64(i64::try_from(n).unwrap_or(i64::MAX)))
}

// ---------------------------------------------------------------------------
// SELECT
// ---------------------------------------------------------------------------

/// Starts a select of `E` rows (every column, in `E::columns()` order).
pub fn select<E: Entity>() -> SelectBuilder<E> {
    SelectBuilder {
        select: Select {
            from: FromClause::Table(E::TABLE),
            projection: E::columns()
                .iter()
                .map(|c| ColumnRef::new(c.name))
                .collect(),
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

// ---------------------------------------------------------------------------
// GET
// ---------------------------------------------------------------------------

/// The row of `E` with the given id, if any. Requires `E: Gettable<E>`.
pub fn get<E: Entity + Gettable<E>>(id: &E::Id) -> Query<Option<E>> {
    let mut builder = select::<E>();
    and_filter(
        &mut builder.select.filter,
        key_filter(E::ID_COLUMNS, id.id_values()),
    );
    builder.optional()
}

/// `id_col_1 = $1 AND id_col_2 = $2 ...`
fn key_filter(columns: &[&'static str], values: Vec<Value>) -> ExprNode {
    debug_assert_eq!(
        columns.len(),
        values.len(),
        "key arity must match ID_COLUMNS"
    );
    columns
        .iter()
        .zip(values)
        .map(|(c, v)| {
            ExprNode::Binary(
                BinOp::Eq,
                Box::new(ExprNode::Column(ColumnRef::new(c))),
                Box::new(ExprNode::Param(v)),
            )
        })
        .reduce(|a, b| ExprNode::Binary(BinOp::And, Box::new(a), Box::new(b)))
        .expect("an entity has at least one id column")
}

// ---------------------------------------------------------------------------
// INSERT
// ---------------------------------------------------------------------------

/// Starts an insert into `E`. Rows are added with values implementing
/// [`Insertable<E>`]: the entity itself, or a dedicated input struct.
pub fn insert<E: Entity>() -> InsertBuilder<E> {
    InsertBuilder { _e: PhantomData }
}

#[derive(Debug, Clone, Copy)]
/// An insert into `E` without rows yet; add them with
/// [`value`](Self::value) or [`values`](Self::values).
pub struct InsertBuilder<E> {
    _e: PhantomData<fn() -> E>,
}

impl<E: Entity> InsertBuilder<E> {
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

impl<E: Entity, V: Insertable<E>> InsertRows<E, V> {
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
        self.insert.returning = Some(
            E::columns()
                .iter()
                .map(|c| ColumnRef::new(c.name))
                .collect(),
        );
        Query::new(Statement::Insert(self.insert), R::EXPECT)
    }

    /// Returns the one inserted row (see [`returning`](Self::returning)).
    pub fn returning_one(self) -> Query<E> {
        self.returning()
    }

    /// Returns every inserted row (see [`returning`](Self::returning)).
    pub fn returning_all(self) -> Query<Vec<E>> {
        self.returning()
    }
}

// ---------------------------------------------------------------------------
// UPDATE
// ---------------------------------------------------------------------------

/// Starts an update of `E` rows; set columns with `one`, `apply`, `set`
/// or `set_expr`, and restrict it with `filter`.
pub fn update<E: Entity>() -> UpdateBuilder<E> {
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

impl<E: Entity> UpdateBuilder<E> {
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

    /// Adds `value`'s assignments without restricting rows; combine with `filter`.
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

// ---------------------------------------------------------------------------
// DELETE
// ---------------------------------------------------------------------------

/// Starts a delete from `E`. Requires `E: Deletable<E>`.
pub fn delete<E: Entity + Deletable<E>>() -> DeleteBuilder<E> {
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
    /// Deletes the row `value` identifies.
    pub fn one<V: Deletable<E>>(mut self, value: &V) -> Self {
        and_filter(
            &mut self.delete.filter,
            key_filter(E::ID_COLUMNS, value.delete_key()),
        );
        self
    }

    /// Deletes the row with id `id`.
    pub fn by_id(mut self, id: &E::Id) -> Self {
        and_filter(
            &mut self.delete.filter,
            key_filter(E::ID_COLUMNS, id.id_values()),
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

// ---------------------------------------------------------------------------
// Raw SQL
// ---------------------------------------------------------------------------

/// Raw SQL with bound parameters. Fragments must be static text; values go
/// through [`RawBuilder::bind`]. The SQL is not portable across dialects.
pub fn raw() -> RawBuilder {
    RawBuilder::default()
}

#[derive(Debug, Clone, Default)]
/// Raw SQL being built from static fragments and bound values.
pub struct RawBuilder {
    raw: RawSql,
}

impl RawBuilder {
    /// Appends a static SQL fragment.
    pub fn sql(mut self, fragment: &'static str) -> Self {
        self.raw.parts.push(RawPart::Sql(fragment));
        self
    }

    /// Appends a bound parameter.
    pub fn bind<T: ScalarColumn>(mut self, value: T) -> Self {
        self.raw.parts.push(RawPart::Param(value.to_value()));
        self
    }

    /// Appends a bound parameter, as an untyped [`Value`].
    pub fn bind_value(mut self, value: Value) -> Self {
        self.raw.parts.push(RawPart::Param(value));
        self
    }

    /// Binds a parameter-like value (`&str`, `&T`, `T`), typed as `B`.
    ///
    /// # Panics
    /// If `value` is an expression rather than a value (a column, say): raw
    /// SQL fragments are opaque, so only values can be bound into them.
    pub fn bind_param<B, V: IntoExpr<B>>(self, value: V) -> Self {
        match value.into_node() {
            ExprNode::Param(v) => self.bind_value(v),
            other => panic!("raw SQL can only bind values, not expressions ({other:?})"),
        }
    }

    /// Finishes as a row-returning query.
    pub fn rows<R: RowsResult>(self) -> Query<R> {
        Query::new(Statement::Raw(self.raw), R::EXPECT)
    }

    /// Finishes as a row-modifying query.
    pub fn execute<R: AffectedResult>(self) -> Query<R> {
        Query::new(Statement::Raw(self.raw), Expect::Affected)
    }
}
