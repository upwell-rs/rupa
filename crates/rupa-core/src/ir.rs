//! Untyped, dialect-free query IR.
//!
//! Invariants:
//! - Data enters only through [`ExprNode::Param`]. Every other string in the IR
//!   (identifiers, operators, function names, JSON keys, raw SQL fragments) is
//!   `&'static str`: author-controlled source text, never runtime input.
//! - DSL function calls stay un-lowered ([`ExprNode::Dsl`]) until rendering, so a
//!   statement can be rendered for any dialect or evaluated by the memory backend.
//! - Column references name their source by [`SourceId`], not by table, and
//!   projections are positional, so joins can be added as new `FromClause` variants.

use std::fmt;

use crate::dialect::Dialect;
use crate::error::DslError;
use crate::value::{SqlType, Value};

#[derive(Debug, Clone, PartialEq)]
pub enum Statement {
    Select(Select),
    /// `SELECT EXISTS (SELECT 1 FROM .. WHERE ..)`; yields one boolean row.
    Exists(Select),
    Insert(Insert),
    Update(Update),
    Delete(Delete),
    Raw(RawSql),
    /// Applies a security context inside the current transaction
    /// (transaction-local settings). Requires `Capability::NativeRls`.
    ApplySecurity(crate::security::SecurityContext),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TableRef {
    pub schema: Option<&'static str>,
    pub name: &'static str,
}

impl TableRef {
    pub const fn new(name: &'static str) -> Self {
        Self { schema: None, name }
    }
    pub const fn with_schema(schema: &'static str, name: &'static str) -> Self {
        Self {
            schema: Some(schema),
            name,
        }
    }
}

/// Index of a row source within a statement. Always `SourceId(0)` until joins exist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SourceId(pub u16);

/// Deliberately exhaustive: adding joins must break every renderer's match.
#[derive(Debug, Clone, PartialEq)]
pub enum FromClause {
    Table(TableRef),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColumnRef {
    pub source: SourceId,
    pub name: &'static str,
}

impl ColumnRef {
    pub const fn new(name: &'static str) -> Self {
        Self {
            source: SourceId(0),
            name,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Select {
    pub from: FromClause,
    /// Rows are decoded by position against this list.
    pub projection: Vec<ColumnRef>,
    pub filter: Option<ExprNode>,
    pub order_by: Vec<OrderBy>,
    pub limit: Option<ExprNode>,
    pub offset: Option<ExprNode>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Insert {
    pub table: TableRef,
    pub columns: Vec<&'static str>,
    pub rows: Vec<Vec<ExprNode>>,
    /// `RETURNING` projection: the inserted rows come back, decoded by position.
    /// Requires [`Capability::Returning`](crate::Capability::Returning).
    pub returning: Option<Vec<ColumnRef>>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Update {
    pub table: TableRef,
    pub set: Vec<(&'static str, ExprNode)>,
    pub filter: Option<ExprNode>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Delete {
    pub table: TableRef,
    pub filter: Option<ExprNode>,
}

/// Raw SQL escape hatch. Fragments are static text; parameters are bound and
/// rendered with the target dialect's placeholder syntax. Not portable across
/// dialects: the SQL text itself is the author's responsibility.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RawSql {
    pub parts: Vec<RawPart>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RawPart {
    Sql(&'static str),
    Param(Value),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Asc,
    Desc,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OrderBy {
    pub expr: ExprNode,
    pub direction: Direction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnOp {
    Not,
    IsNull,
    IsNotNull,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    Like,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathSeg {
    Key(&'static str),
    Index(u32),
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExprNode {
    Column(ColumnRef),
    Param(Value),
    Unary(UnOp, Box<ExprNode>),
    Binary(BinOp, Box<ExprNode>, Box<ExprNode>),
    In {
        expr: Box<ExprNode>,
        list: Vec<ExprNode>,
        negated: bool,
    },
    /// Path into a JSON column. `as_text` selects a text result (`->>`) over JSON (`->`).
    JsonPath {
        column: ColumnRef,
        path: Vec<PathSeg>,
        as_text: bool,
    },
    Cast(Box<ExprNode>, SqlType),
    Dsl(DslCall),
    /// Infix operator; produced by DSL lowering.
    RawOp(&'static str, Box<ExprNode>, Box<ExprNode>),
    /// Function call; produced by DSL lowering.
    Call(&'static str, Vec<ExprNode>),
}

/// Lowers a DSL call for a concrete dialect. This is the `#[dsl::function]` body.
pub type LowerFn = fn(&dyn Dialect, Vec<ExprNode>) -> Result<ExprNode, DslError>;
/// Evaluates a DSL call over values, for the memory backend.
pub type EvalFn = fn(&[Value]) -> Result<Value, DslError>;

/// Static definition of a DSL function.
pub struct DslFnDef {
    pub name: &'static str,
    pub lower: LowerFn,
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
pub struct DslCall {
    pub def: &'static DslFnDef,
    pub args: Vec<ExprNode>,
}

impl DslCall {
    pub fn lower(&self, dialect: &dyn Dialect) -> Result<ExprNode, DslError> {
        (self.def.lower)(dialect, self.args.clone())
    }
}

impl PartialEq for DslCall {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self.def, other.def) && self.args == other.args
    }
}

/// Transaction isolation level. Dialects without a level reject it when rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IsolationLevel {
    ReadUncommitted,
    ReadCommitted,
    RepeatableRead,
    Serializable,
}

/// Options for starting a top-level transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TxOptions {
    pub isolation: Option<IsolationLevel>,
    pub read_only: bool,
}

impl TxOptions {
    pub fn isolation(mut self, level: IsolationLevel) -> Self {
        self.isolation = Some(level);
        self
    }

    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Transaction control. Depths start at 1 for the first savepoint inside a
/// transaction; depth 0 is the transaction itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxStatement {
    Begin(TxOptions),
    Commit,
    Rollback,
    Savepoint(u32),
    ReleaseSavepoint(u32),
    /// Roll back to the savepoint and release it.
    RollbackToSavepoint(u32),
}
