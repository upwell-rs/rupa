//! Raw SQL with bound parameters.

use crate::expr::IntoExpr;
use crate::ir::{ExprNode, RawPart, RawSql, Statement};
use crate::query::{AffectedResult, Expect, Query, RowsResult};
use crate::value::{ScalarColumn, Value};

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
