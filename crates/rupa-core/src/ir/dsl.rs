//! DSL function definitions and calls.

use std::fmt;

use super::ExprNode;
use crate::dialect::Dialect;
use crate::error::DslError;
use crate::value::Value;

/// Lowers a DSL call for a concrete dialect. This is the `#[dsl::function]` body.
pub type LowerFn = fn(&dyn Dialect, Vec<ExprNode>) -> Result<ExprNode, DslError>;
/// Evaluates a DSL call over values, for the memory backend.
pub type EvalFn = fn(&[Value]) -> Result<Value, DslError>;

/// Static definition of a DSL function.
pub struct DslFnDef {
    /// The function's name, for errors.
    pub name: &'static str,
    /// Lowers a call to plain IR for a dialect.
    pub lower: LowerFn,
    /// Evaluates a call on values, for the memory backend.
    pub eval: Option<EvalFn>,
}

impl fmt::Debug for DslFnDef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DslFnDef")
            .field("name", &self.name)
            .field("eval", &self.eval.is_some())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone)]
/// A call of a DSL function.
pub struct DslCall {
    /// The function.
    pub def: &'static DslFnDef,
    /// The arguments.
    pub args: Vec<ExprNode>,
}

impl DslCall {
    /// Lowers the call to plain IR for `dialect`.
    pub fn lower(&self, dialect: &dyn Dialect) -> Result<ExprNode, DslError> {
        (self.def.lower)(dialect, self.args.clone())
    }
}

impl PartialEq for DslCall {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.def, other.def) && self.args == other.args
    }
}
