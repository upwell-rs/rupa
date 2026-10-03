//! Typed statement builders. Each produces a dialect-free [`Query<R>`]; no IO.

use std::marker::PhantomData;

use crate::column::{Column, Scalar};
use crate::entity::Entity;
use crate::expr::IntoExpr;
use crate::ir::{
    BinOp, ColumnRef, Delete, ExprNode, FromClause, Insert, OrderBy, RawPart, RawSql, Select,
    Statement, Update,
};
use crate::query::{AffectedResult, Expect, Query, RowsResult};
use crate::value::{ScalarColumn, Value};

fn and_filter(filter: &mut Option<ExprNode>, cond: ExprNode) {
    *filter = Some(match filter.take() {
        None => cond,
        Some(prev) => ExprNode::Binary(BinOp::And, Box::new(prev), Box::new(cond)),
    });
}

fn count_param(n: u64) -> ExprNode {
    // SQL LIMIT/OFFSET are signed 64-bit; anything larger means "no bound".
    ExprNode::Param(Value::I64(i64::try_from(n).unwrap_or(i64::MAX)))
}

// ---------------------------------------------------------------------------
// SELECT
// ---------------------------------------------------------------------------

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
pub struct SelectBuilder<E> {
    select: Select,
    _e: PhantomData<fn() -> E>,
}

impl<E: Entity> SelectBuilder<E> {
    /// Adds a condition; repeated calls are ANDed.
    pub fn filter(mut self, cond: impl IntoExpr<bool>) -> Self {
        and_filter(&mut self.select.filter, cond.into_node());
        self
    }

    pub fn order_by(mut self, order: OrderBy) -> Self {
        self.select.order_by.push(order);
        self
    }

    pub fn limit(mut self, n: u64) -> Self {
        self.select.limit = Some(count_param(n));
        self
    }

    pub fn offset(mut self, n: u64) -> Self {
        self.select.offset = Some(count_param(n));
        self
    }

    /// Builds with the result shape taken from `R`; used by generated code,
    /// which reads `R` off the method's return type.
    pub fn build<R: RowsResult<Item = E>>(self) -> Query<R> {
        Query::new(Statement::Select(self.select), R::EXPECT)
    }

    pub fn all(self) -> Query<Vec<E>> {
        self.build()
    }

    pub fn optional(self) -> Query<Option<E>> {
        self.build()
    }

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
// INSERT
// ---------------------------------------------------------------------------

pub fn insert<E: Entity>() -> InsertBuilder<E> {
    InsertBuilder {
        insert: Insert {
            table: E::TABLE,
            columns: E::columns().iter().map(|c| c.name).collect(),
            rows: Vec::new(),
        },
        _e: PhantomData,
    }
}

#[derive(Debug, Clone)]
pub struct InsertBuilder<E> {
    insert: Insert,
    _e: PhantomData<fn() -> E>,
}

impl<E: Entity> InsertBuilder<E> {
    pub fn value(mut self, entity: &E) -> Self {
        self.insert.rows.push(
            entity
                .to_values()
                .into_iter()
                .map(ExprNode::Param)
                .collect(),
        );
        self
    }

    pub fn values<'a>(self, entities: impl IntoIterator<Item = &'a E>) -> Self {
        entities.into_iter().fold(self, Self::value)
    }

    pub fn build<R: AffectedResult>(self) -> Query<R> {
        Query::new(Statement::Insert(self.insert), Expect::Affected)
    }

    pub fn affected(self) -> Query<u64> {
        self.build()
    }
}

// ---------------------------------------------------------------------------
// UPDATE
// ---------------------------------------------------------------------------

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
pub struct UpdateBuilder<E> {
    update: Update,
    _e: PhantomData<fn() -> E>,
}

impl<E: Entity> UpdateBuilder<E> {
    /// Sets a column of `E` to a field value, encoded as the entity stores it
    /// (scalar or JSON).
    pub fn set<T, K>(mut self, column: Column<E, T, K>, value: &T) -> Self {
        self.update
            .set
            .push((column.name(), ExprNode::Param(column.encode(value))));
        self
    }

    /// Sets a scalar column of `E` to an expression, e.g. another column.
    pub fn set_expr<T: ScalarColumn>(
        mut self,
        column: Column<E, T, Scalar>,
        expr: impl IntoExpr<T::Base>,
    ) -> Self {
        self.update.set.push((column.name(), expr.into_node()));
        self
    }

    pub fn filter(mut self, cond: impl IntoExpr<bool>) -> Self {
        and_filter(&mut self.update.filter, cond.into_node());
        self
    }

    pub fn build<R: AffectedResult>(self) -> Query<R> {
        Query::new(Statement::Update(self.update), Expect::Affected)
    }

    pub fn affected(self) -> Query<u64> {
        self.build()
    }
}

// ---------------------------------------------------------------------------
// DELETE
// ---------------------------------------------------------------------------

pub fn delete<E: Entity>() -> DeleteBuilder<E> {
    DeleteBuilder {
        delete: Delete {
            table: E::TABLE,
            filter: None,
        },
        _e: PhantomData,
    }
}

#[derive(Debug, Clone)]
pub struct DeleteBuilder<E> {
    delete: Delete,
    _e: PhantomData<fn() -> E>,
}

impl<E: Entity> DeleteBuilder<E> {
    pub fn filter(mut self, cond: impl IntoExpr<bool>) -> Self {
        and_filter(&mut self.delete.filter, cond.into_node());
        self
    }

    pub fn build<R: AffectedResult>(self) -> Query<R> {
        Query::new(Statement::Delete(self.delete), Expect::Affected)
    }

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
pub struct RawBuilder {
    raw: RawSql,
}

impl RawBuilder {
    pub fn sql(mut self, fragment: &'static str) -> Self {
        self.raw.parts.push(RawPart::Sql(fragment));
        self
    }

    pub fn bind<T: ScalarColumn>(mut self, value: T) -> Self {
        self.raw.parts.push(RawPart::Param(value.to_value()));
        self
    }

    pub fn bind_value(mut self, value: Value) -> Self {
        self.raw.parts.push(RawPart::Param(value));
        self
    }

    pub fn rows<R: RowsResult>(self) -> Query<R> {
        Query::new(Statement::Raw(self.raw), R::EXPECT)
    }

    pub fn execute<R: AffectedResult>(self) -> Query<R> {
        Query::new(Statement::Raw(self.raw), Expect::Affected)
    }
}
