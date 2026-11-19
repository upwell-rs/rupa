use super::{IndexKind, IndexableColumn};
use crate::column::{Column, Json, Scalar};
use crate::entity::Relation;
use crate::expr::Expr;
use crate::ir::TableRef;
use crate::schema::{ColumnDef, ColumnType, DdlExpr};
use crate::value::ScalarColumn;

/// The type an expression evaluates to, as an index part: the kind of column
/// it stands in for ([`Scalar`] or [`Json`]) and its column type.
pub trait ExprResult: 'static {
    /// The column kind the result is indexed as.
    type Kind;

    /// The column type of the result.
    const COLUMN_TYPE: ColumnType;
}

impl<T: ScalarColumn> ExprResult for T {
    type Kind = Scalar;

    const COLUMN_TYPE: ColumnType = T::COLUMN_TYPE;
}

impl ExprResult for serde_json::Value {
    type Kind = Json;

    const COLUMN_TYPE: ColumnType = ColumnType::Json;
}

impl ExprResult for Option<serde_json::Value> {
    type Kind = Json;

    const COLUMN_TYPE: ColumnType = ColumnType::Json;
}

/// An expression that kind `K` can index, by the same rule as a column of
/// its result type.
#[diagnostic::on_unimplemented(
    message = "an expression of type `{Self}` cannot be indexed by `{K}`",
    label = "not accepted by this index kind",
    note = "scalar types need `IndexableBy<{K}>`; JSON results need `{K}: JsonIndexKind`"
)]
pub trait IndexableExpr<K: IndexKind> {
    /// The index part, scoped to the columns of entity `E`.
    fn into_part<E: Relation>(self) -> IndexExpr;
}

impl<K: IndexKind, T: ExprResult> IndexableExpr<K> for Expr<T>
where
    T::Kind: IndexableColumn<K, T>,
{
    fn into_part<E: Relation>(self) -> IndexExpr {
        IndexExpr {
            expr: DdlExpr::of::<E, T>(self),
            ty: T::COLUMN_TYPE,
        }
    }
}

/// An indexed column: its name and type, so rendering can check the type
/// against the dialect, and the table its handle was declared for, so
/// rendering can refuse a column of another table. The table is not part of
/// the checksum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct IndexColumn {
    /// The column name.
    pub name: &'static str,
    /// The column type, with its length for bounded text.
    pub ty: ColumnType,
    /// The table the column's handle was declared for, if known. Rendering
    /// refuses a column recorded for a table other than the indexed one.
    pub table: Option<TableRef>,
}

impl IndexColumn {
    /// A column named `name` of type `ty`, of no known table.
    pub const fn new(name: &'static str, ty: ColumnType) -> Self {
        Self {
            name,
            ty,
            table: None,
        }
    }

    /// The same column, recorded for `table`: rendering then refuses it in
    /// an index or unique of any other table.
    pub const fn in_table(mut self, table: TableRef) -> Self {
        self.table = Some(table);

        self
    }

    /// The column a field handle of relation `E` describes, recording the
    /// handle's table, or `E`'s when the handle has none.
    pub fn of_relation<E: Relation, T, K>(column: Column<E, T, K>) -> Self {
        let table = column.table().unwrap_or(E::TABLE);

        Self::of(column).in_table(table)
    }

    /// The column a field handle describes, recording the handle's table.
    pub const fn of<E, T, K>(column: Column<E, T, K>) -> Self {
        Self {
            table: column.table(),
            ..Self::new(column.name(), column.column_type())
        }
    }
}

impl From<&ColumnDef> for IndexColumn {
    fn from(column: &ColumnDef) -> Self {
        Self::new(column.name, column.ty)
    }
}

/// An indexed expression and the type it evaluates to. MySQL refuses
/// expressions of unbounded type, which only rendering can tell.
#[derive(Debug, Clone, PartialEq)]
pub struct IndexExpr {
    /// The expression.
    pub expr: DdlExpr,
    /// The type the expression evaluates to.
    pub ty: ColumnType,
}

/// One entry of an index key: a column or an expression. Deliberately
/// exhaustive, like [`SchemaOp`](crate::schema::SchemaOp): a new kind of part
/// must break every renderer's and every checksum encoder's match.
#[derive(Debug, Clone, PartialEq)]
pub enum IndexPart {
    /// A column.
    Column(IndexColumn),
    /// An expression over the table's columns.
    Expr(IndexExpr),
}

impl From<IndexColumn> for IndexPart {
    fn from(column: IndexColumn) -> Self {
        Self::Column(column)
    }
}

impl From<IndexExpr> for IndexPart {
    fn from(expr: IndexExpr) -> Self {
        Self::Expr(expr)
    }
}
