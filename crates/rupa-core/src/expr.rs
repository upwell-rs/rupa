//! Typed expressions over the untyped IR.
//!
//! `Expr<T>` is an [`ExprNode`] tagged with the Rust type it evaluates to.
//! Comparisons take `impl IntoExpr<Base>` where `Base` is the column type with
//! nullability stripped (`i64` for both `i64` and `Option<i64>`). That way a
//! parameter's type is checked against its column by the compiler.

use std::fmt;
use std::marker::PhantomData;
use std::ops::Not;

use crate::column::{Column, IsJson, Json, Scalar};
use crate::ir::{BinOp, ColumnRef, DslCall, DslFnDef, ExprNode, OrderBy, PathSeg, UnOp};
use crate::value::ScalarColumn;

pub struct Expr<T> {
    node: ExprNode,
    _t: PhantomData<fn() -> T>,
}

impl<T> Clone for Expr<T> {
    fn clone(&self) -> Self {
        Self::from_node(self.node.clone())
    }
}

impl<T> fmt::Debug for Expr<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.node.fmt(f)
    }
}

impl<T> Expr<T> {
    /// Wraps an untyped node. The caller asserts it evaluates to `T`; intended
    /// for DSL function authors and generated code.
    pub fn from_node(node: ExprNode) -> Self {
        Self {
            node,
            _t: PhantomData,
        }
    }

    pub fn node(&self) -> &ExprNode {
        &self.node
    }

    pub fn into_node(self) -> ExprNode {
        self.node
    }

    /// Infix operator. `op` is static source text; operands are expressions
    /// (and therefore bound parameters when they carry data).
    pub fn raw_op<A, B>(op: &'static str, lhs: Expr<A>, rhs: Expr<B>) -> Self {
        Self::from_node(ExprNode::RawOp(op, Box::new(lhs.node), Box::new(rhs.node)))
    }

    /// SQL function call by static name.
    pub fn call<A, const N: usize>(name: &'static str, args: [Expr<A>; N]) -> Self {
        Self::from_node(ExprNode::Call(
            name,
            args.into_iter().map(Expr::into_node).collect(),
        ))
    }

    /// Un-lowered DSL function call; lowered per dialect at render time.
    pub fn dsl(def: &'static DslFnDef, args: Vec<ExprNode>) -> Self {
        Self::from_node(ExprNode::Dsl(DslCall { def, args }))
    }
}

/// Anything usable where an expression of type `T` is expected: expressions,
/// scalar column handles, and plain Rust values (bound as parameters).
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be used as an expression of type `{T}`",
    label = "expected a `{T}` value, column or expression"
)]
pub trait IntoExpr<T> {
    fn into_node(self) -> ExprNode;
}

impl<T: ScalarColumn> IntoExpr<T::Base> for Expr<T> {
    fn into_node(self) -> ExprNode {
        self.node
    }
}

impl<E, T: ScalarColumn> IntoExpr<T::Base> for Column<E, T, Scalar> {
    fn into_node(self) -> ExprNode {
        ExprNode::Column(self.column_ref())
    }
}

impl<T: ScalarColumn> IntoExpr<T> for T {
    fn into_node(self) -> ExprNode {
        ExprNode::Param(self.to_value())
    }
}

impl<T: ScalarColumn> IntoExpr<T> for &T {
    fn into_node(self) -> ExprNode {
        ExprNode::Param(self.to_value())
    }
}

impl IntoExpr<String> for &str {
    fn into_node(self) -> ExprNode {
        ExprNode::Param(crate::value::Value::Text(self.to_owned()))
    }
}

/// Binds a value as a parameter expression.
pub fn bind<T: ScalarColumn>(v: T) -> Expr<T> {
    Expr::from_node(ExprNode::Param(v.to_value()))
}

type Base<T> = <T as ScalarColumn>::Base;

fn binary(op: BinOp, lhs: ExprNode, rhs: ExprNode) -> Expr<bool> {
    Expr::from_node(ExprNode::Binary(op, Box::new(lhs), Box::new(rhs)))
}

/// Operations on scalar expressions and scalar column handles.
#[allow(
    clippy::wrong_self_convention,
    reason = "builder methods consume their operand, like every other operator here"
)]
pub trait ExprOps: Sized {
    type Ty: ScalarColumn;

    fn into_expr_node(self) -> ExprNode;

    fn eq(self, rhs: impl IntoExpr<Base<Self::Ty>>) -> Expr<bool> {
        binary(BinOp::Eq, self.into_expr_node(), rhs.into_node())
    }
    fn ne(self, rhs: impl IntoExpr<Base<Self::Ty>>) -> Expr<bool> {
        binary(BinOp::Ne, self.into_expr_node(), rhs.into_node())
    }
    fn lt(self, rhs: impl IntoExpr<Base<Self::Ty>>) -> Expr<bool> {
        binary(BinOp::Lt, self.into_expr_node(), rhs.into_node())
    }
    fn le(self, rhs: impl IntoExpr<Base<Self::Ty>>) -> Expr<bool> {
        binary(BinOp::Le, self.into_expr_node(), rhs.into_node())
    }
    fn gt(self, rhs: impl IntoExpr<Base<Self::Ty>>) -> Expr<bool> {
        binary(BinOp::Gt, self.into_expr_node(), rhs.into_node())
    }
    fn ge(self, rhs: impl IntoExpr<Base<Self::Ty>>) -> Expr<bool> {
        binary(BinOp::Ge, self.into_expr_node(), rhs.into_node())
    }

    fn like(self, pattern: impl IntoExpr<String>) -> Expr<bool>
    where
        Self::Ty: ScalarColumn<Base = String>,
    {
        binary(BinOp::Like, self.into_expr_node(), pattern.into_node())
    }

    fn in_<I>(self, items: I) -> Expr<bool>
    where
        I: IntoIterator,
        I::Item: IntoExpr<Base<Self::Ty>>,
    {
        in_list(self.into_expr_node(), items, false)
    }

    fn not_in<I>(self, items: I) -> Expr<bool>
    where
        I: IntoIterator,
        I::Item: IntoExpr<Base<Self::Ty>>,
    {
        in_list(self.into_expr_node(), items, true)
    }

    fn is_null(self) -> Expr<bool> {
        Expr::from_node(ExprNode::Unary(
            UnOp::IsNull,
            Box::new(self.into_expr_node()),
        ))
    }
    fn is_not_null(self) -> Expr<bool> {
        Expr::from_node(ExprNode::Unary(
            UnOp::IsNotNull,
            Box::new(self.into_expr_node()),
        ))
    }

    fn asc(self) -> OrderBy {
        OrderBy {
            expr: self.into_expr_node(),
            direction: crate::ir::Direction::Asc,
        }
    }
    fn desc(self) -> OrderBy {
        OrderBy {
            expr: self.into_expr_node(),
            direction: crate::ir::Direction::Desc,
        }
    }
}

fn in_list<B, I>(expr: ExprNode, items: I, negated: bool) -> Expr<bool>
where
    I: IntoIterator,
    I::Item: IntoExpr<B>,
{
    let list = items.into_iter().map(IntoExpr::into_node).collect();
    Expr::from_node(ExprNode::In {
        expr: Box::new(expr),
        list,
        negated,
    })
}

impl<T: ScalarColumn> ExprOps for Expr<T> {
    type Ty = T;
    fn into_expr_node(self) -> ExprNode {
        self.node
    }
}

impl<E, T: ScalarColumn> ExprOps for Column<E, T, Scalar> {
    type Ty = T;
    fn into_expr_node(self) -> ExprNode {
        ExprNode::Column(self.column_ref())
    }
}

impl<T: ScalarColumn<Base = bool>> Expr<T> {
    pub fn and(self, rhs: impl IntoExpr<bool>) -> Expr<bool> {
        binary(BinOp::And, self.node, rhs.into_node())
    }
    pub fn or(self, rhs: impl IntoExpr<bool>) -> Expr<bool> {
        binary(BinOp::Or, self.node, rhs.into_node())
    }
}

impl<T: ScalarColumn<Base = bool>> Not for Expr<T> {
    type Output = Expr<bool>;
    fn not(self) -> Expr<bool> {
        Expr::from_node(ExprNode::Unary(UnOp::Not, Box::new(self.node)))
    }
}

// ---------------------------------------------------------------------------
// JSON columns
// ---------------------------------------------------------------------------

impl<E, T, K> Column<E, T, K> {
    /// Starts a JSON path at key `key`. Only available on JSON columns.
    pub fn path(self, key: &'static str) -> JsonPath
    where
        K: IsJson,
    {
        JsonPath {
            column: self.column_ref(),
            path: vec![PathSeg::Key(key)],
        }
    }
}

// A separate impl so these don't shadow `ExprOps` on scalar columns.
impl<E, T> Column<E, T, Json> {
    /// SQL `NULL` test (not JSON `null`).
    pub fn is_null(self) -> Expr<bool> {
        Expr::from_node(ExprNode::Unary(
            UnOp::IsNull,
            Box::new(ExprNode::Column(self.column_ref())),
        ))
    }

    pub fn is_not_null(self) -> Expr<bool> {
        Expr::from_node(ExprNode::Unary(
            UnOp::IsNotNull,
            Box::new(ExprNode::Column(self.column_ref())),
        ))
    }
}

/// A path into a JSON column. Missing keys yield SQL `NULL`, so every
/// projection out of a path is nullable.
#[derive(Debug, Clone, PartialEq)]
pub struct JsonPath {
    column: ColumnRef,
    path: Vec<PathSeg>,
}

impl JsonPath {
    pub fn key(mut self, key: &'static str) -> Self {
        self.path.push(PathSeg::Key(key));
        self
    }

    pub fn index(mut self, index: u32) -> Self {
        self.path.push(PathSeg::Index(index));
        self
    }

    /// The value at this path as text.
    pub fn text(self) -> Expr<Option<String>> {
        Expr::from_node(self.node(true))
    }

    /// The value at this path as JSON.
    pub fn json(self) -> Expr<Option<serde_json::Value>> {
        Expr::from_node(self.node(false))
    }

    /// The value at this path, cast to a scalar SQL type.
    pub fn cast<T: ScalarColumn>(self) -> Expr<Option<T::Base>> {
        Expr::from_node(ExprNode::Cast(Box::new(self.node(true)), T::SQL_TYPE))
    }

    fn node(self, as_text: bool) -> ExprNode {
        ExprNode::JsonPath {
            column: self.column,
            path: self.path,
            as_text,
        }
    }
}
