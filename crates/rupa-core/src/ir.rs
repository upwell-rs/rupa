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
/// One statement.
pub enum Statement {
    /// `SELECT`.
    Select(Select),
    /// `SELECT EXISTS (SELECT 1 FROM .. WHERE ..)`; yields one boolean row.
    Exists(Select),
    /// `INSERT`.
    Insert(Insert),
    /// `UPDATE`.
    Update(Update),
    /// `DELETE`.
    Delete(Delete),
    /// Raw SQL: static fragments and bound parameters.
    Raw(RawSql),
    /// Applies a security context inside the current transaction
    /// (transaction-local settings). Requires `Capability::NativeRls`.
    ApplySecurity(crate::security::SecurityContext),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// A table, optionally schema-qualified.
pub struct TableRef {
    /// The schema (`None`: the connection's default).
    pub schema: Option<&'static str>,
    /// The table name.
    pub name: &'static str,
}

impl TableRef {
    /// A table in the default schema.
    pub const fn new(name: &'static str) -> Self {
        Self { schema: None, name }
    }
    /// A table in `schema`.
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
    /// A single table.
    Table(TableRef),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// A column of one of the statement's row sources.
pub struct ColumnRef {
    /// Which source (always the first, until joins exist).
    pub source: SourceId,
    /// The column name.
    pub name: &'static str,
}

impl ColumnRef {
    /// A column of the statement's (only) table.
    pub const fn new(name: &'static str) -> Self {
        Self {
            source: SourceId(0),
            name,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
/// A `SELECT`.
pub struct Select {
    /// The rows' source.
    pub from: FromClause,
    /// Rows are decoded by position against this list.
    pub projection: Vec<ColumnRef>,
    /// The `WHERE` condition.
    pub filter: Option<ExprNode>,
    /// Sort keys, in order.
    pub order_by: Vec<OrderBy>,
    /// `LIMIT` (a bound integer).
    pub limit: Option<ExprNode>,
    /// `OFFSET` (a bound integer).
    pub offset: Option<ExprNode>,
}

#[derive(Debug, Clone, PartialEq)]
/// An `INSERT` of one or more rows with the same columns.
pub struct Insert {
    /// The target table.
    pub table: TableRef,
    /// The columns every row sets.
    pub columns: Vec<&'static str>,
    /// The rows' values, in `columns` order.
    pub rows: Vec<Vec<ExprNode>>,
    /// `RETURNING` projection: the inserted rows come back, decoded by position.
    /// Requires [`Capability::Returning`](crate::Capability::Returning).
    pub returning: Option<Vec<ColumnRef>>,
}

#[derive(Debug, Clone, PartialEq)]
/// An `UPDATE`.
pub struct Update {
    /// The target table.
    pub table: TableRef,
    /// `column = expression` assignments.
    pub set: Vec<(&'static str, ExprNode)>,
    /// The `WHERE` condition (`None`: every row).
    pub filter: Option<ExprNode>,
}

#[derive(Debug, Clone, PartialEq)]
/// A `DELETE`.
pub struct Delete {
    /// The target table.
    pub table: TableRef,
    /// The `WHERE` condition (`None`: every row).
    pub filter: Option<ExprNode>,
}

/// Raw SQL escape hatch. Fragments are static text; parameters are bound and
/// rendered with the target dialect's placeholder syntax. Not portable across
/// dialects: the SQL text itself is the author's responsibility.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RawSql {
    /// The fragments and parameters, in order.
    pub parts: Vec<RawPart>,
}

#[derive(Debug, Clone, PartialEq)]
/// A piece of raw SQL.
pub enum RawPart {
    /// Static SQL text.
    Sql(&'static str),
    /// A bound parameter.
    Param(Value),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Sort direction.
pub enum Direction {
    /// Ascending, `NULL`s last (as Postgres).
    Asc,
    /// Descending, `NULL`s first (as Postgres).
    Desc,
}

#[derive(Debug, Clone, PartialEq)]
/// A sort key.
pub struct OrderBy {
    /// The expression sorted by.
    pub expr: ExprNode,
    /// Its direction.
    pub direction: Direction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Unary operators.
pub enum UnOp {
    /// `NOT`.
    Not,
    /// `IS NULL`.
    IsNull,
    /// `IS NOT NULL`.
    IsNotNull,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Binary operators.
pub enum BinOp {
    /// `=`.
    Eq,
    /// `<>`.
    Ne,
    /// `<`.
    Lt,
    /// `<=`.
    Le,
    /// `>`.
    Gt,
    /// `>=`.
    Ge,
    /// `AND`.
    And,
    /// `OR`.
    Or,
    /// `LIKE` (case-sensitive, `\` escapes).
    Like,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// A step of a JSON path.
pub enum PathSeg {
    /// An object key.
    Key(&'static str),
    /// An array index.
    Index(u32),
}

#[derive(Debug, Clone, PartialEq)]
/// An expression.
pub enum ExprNode {
    /// A column.
    Column(ColumnRef),
    /// A bound parameter: the only way data enters the IR.
    Param(Value),
    /// A unary operator applied to an operand.
    Unary(UnOp, Box<ExprNode>),
    /// A binary operator applied to two operands.
    Binary(BinOp, Box<ExprNode>, Box<ExprNode>),
    /// `expr [NOT] IN (list..)`.
    In {
        /// The tested expression.
        expr: Box<ExprNode>,
        /// The candidates.
        list: Vec<ExprNode>,
        /// `NOT IN` when set.
        negated: bool,
    },
    /// Path into a JSON column. `as_text` selects a text result (`->>`) over JSON (`->`).
    JsonPath {
        /// The JSON column.
        column: ColumnRef,
        /// The path into it.
        path: Vec<PathSeg>,
        /// Whether the result is text (`->>`) rather than JSON (`->`).
        as_text: bool,
    },
    /// A cast to a SQL type.
    Cast(Box<ExprNode>, SqlType),
    /// A DSL function call, lowered at render time.
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

/// Transaction isolation level. Dialects without a level reject it when rendered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum IsolationLevel {
    /// `READ UNCOMMITTED`.
    ReadUncommitted,
    /// `READ COMMITTED`.
    ReadCommitted,
    /// `REPEATABLE READ`.
    RepeatableRead,
    /// `SERIALIZABLE`.
    Serializable,
}

/// Options for starting a top-level transaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TxOptions {
    /// The isolation level (`None`: the database's default).
    pub isolation: Option<IsolationLevel>,
    /// Whether the transaction may only read.
    pub read_only: bool,
}

impl TxOptions {
    /// Sets the isolation level.
    pub fn isolation(mut self, level: IsolationLevel) -> Self {
        self.isolation = Some(level);
        self
    }

    /// Makes the transaction read-only.
    pub fn read_only(mut self) -> Self {
        self.read_only = true;
        self
    }

    /// Whether these are the default options (which savepoints require).
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Transaction control. Depths start at 1 for the first savepoint inside a
/// transaction; depth 0 is the transaction itself.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TxStatement {
    /// Starts a transaction.
    Begin(TxOptions),
    /// Commits the transaction.
    Commit,
    /// Rolls the transaction back.
    Rollback,
    /// Creates savepoint `n`.
    Savepoint(u32),
    /// Releases (keeps the work of) savepoint `n`.
    ReleaseSavepoint(u32),
    /// Roll back to the savepoint and release it.
    RollbackToSavepoint(u32),
}
