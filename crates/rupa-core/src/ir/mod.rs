//! Untyped, dialect-free query IR.
//!
//! Invariants:
//! - Data enters only through [`ExprNode::Param`]. Every other string in the IR
//!   (identifiers, operators, function names, JSON keys, raw SQL fragments) is
//!   `&'static str`: author-controlled source text, never runtime input.
//! - DSL function calls stay un-lowered ([`ExprNode::Dsl`]) until rendering, so a
//!   statement can be rendered for any dialect or evaluated by the memory backend.
//! - Column references name their source by [`SourceId`], not by table, and
//!   projections are positional. Joins number their sources in order
//!   ([`FromClause::Join`]).

mod dsl;
mod tx;

use crate::value::{SqlType, Value};

pub use dsl::{DslCall, DslFnDef, EvalFn, LowerFn};
pub use tx::{IsolationLevel, TxOptions, TxStatement};

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

/// Index of a row source within a statement: the base table is `SourceId(0)`,
/// the n-th join `SourceId(n)`. Statements without joins only use `SourceId(0)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SourceId(pub u16);

/// Deliberately exhaustive: a new kind of row source must break every
/// renderer's match.
#[derive(Debug, Clone, PartialEq)]
pub enum FromClause {
    /// A single table.
    Table(TableRef),
    /// A base table (`SourceId(0)`) joined with further tables, which are
    /// numbered `SourceId(1)`, `SourceId(2)`, .. in order.
    Join {
        /// The first source.
        base: TableRef,
        /// The joined sources, in order.
        joins: Vec<Join>,
    },
}

impl FromClause {
    /// The number of row sources.
    pub fn source_count(&self) -> usize {
        match self {
            FromClause::Table(_) => 1,
            FromClause::Join { joins, .. } => 1 + joins.len(),
        }
    }

    /// The table of source `id`, if it exists.
    pub fn source(&self, id: SourceId) -> Option<TableRef> {
        match (self, id.0) {
            (FromClause::Table(t), 0) | (FromClause::Join { base: t, .. }, 0) => Some(*t),
            (FromClause::Table(_), _) => None,
            (FromClause::Join { joins, .. }, n) => joins.get(usize::from(n) - 1).map(|j| j.table),
        }
    }
}

/// Deliberately exhaustive, like [`FromClause`]: `RIGHT`/`FULL` would need
/// capabilities (MySQL has no `FULL JOIN`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum JoinKind {
    /// `INNER JOIN`: only rows with a match.
    Inner,
    /// `LEFT JOIN`: every row of the earlier sources; the joined source's
    /// columns are `NULL` where nothing matches.
    Left,
}

#[derive(Debug, Clone, PartialEq)]
/// One joined source.
pub struct Join {
    /// Inner or left.
    pub kind: JoinKind,
    /// The joined table.
    pub table: TableRef,
    /// The join condition. It may refer to this source and every earlier one.
    pub on: ExprNode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
/// A column of one of the statement's row sources.
pub struct ColumnRef {
    /// Which source.
    pub source: SourceId,
    /// The column name.
    pub name: &'static str,
    /// The table the column belongs to, when it came from an entity's typed
    /// handle. Rendering, caching and evaluation never read it: it lets a
    /// schema object refuse a column of another table.
    pub table: Option<TableRef>,
}

impl ColumnRef {
    /// A column of the statement's first (or only) table.
    pub const fn new(name: &'static str) -> Self {
        Self::of(SourceId(0), name)
    }

    /// A column of source `source`.
    pub const fn of(source: SourceId, name: &'static str) -> Self {
        Self {
            source,
            name,
            table: None,
        }
    }

    /// The same column, recorded as belonging to `table`.
    pub const fn in_table(mut self, table: TableRef) -> Self {
        self.table = Some(table);

        self
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
