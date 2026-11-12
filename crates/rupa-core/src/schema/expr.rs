//! Expressions inside schema objects (index parts and predicates, and
//! later constraints), rendered in DDL mode: literals are written inline,
//! never bound.

use crate::column::ColumnMeta;
use crate::entity::Entity;
use crate::expr::Expr;
use crate::ir::ExprNode;

/// A query expression used in a schema object. It carries the columns of the
/// entity it was written for, so rendering can refuse a column the table does
/// not have; an expression written without an entity cannot be checked and is
/// left to the database.
#[derive(Debug, Clone, PartialEq)]
pub struct DdlExpr {
    /// The expression.
    pub node: ExprNode,
    /// The columns the expression may name, if known.
    pub scope: Option<&'static [ColumnMeta]>,
}

impl DdlExpr {
    /// An expression without a known table.
    pub const fn new(node: ExprNode) -> Self {
        Self { node, scope: None }
    }

    /// An expression over the columns of entity `E`.
    pub fn of<E: Entity, T>(expr: Expr<T>) -> Self {
        Self {
            node: expr.into_node(),
            scope: Some(E::COLUMNS),
        }
    }
}
