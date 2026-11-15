//! Expressions of schema objects, shared by everything that holds one
//! (index parts and predicates, and constraints).

use rupa_core::ir::TableRef;
use rupa_core::schema::{ColumnDef, DdlExpr, Index, IndexColumn};

use super::{Ddl, ExprRefusal, SchemaError};
use crate::RenderError;
use crate::writer::{DdlMode, Writer};

impl Ddl<'_> {
    /// Writes `expr`, held by the schema object `object` of `table`, in DDL
    /// mode:
    /// values inline as checked literals, restricted to what DDL can hold
    /// (see [`ExprRefusal`](super::ExprRefusal)).
    pub(super) fn expr(
        &mut self,
        object: &'static str,
        table: TableRef,
        expr: &DdlExpr,
    ) -> Result<(), RenderError> {
        self.restricted_expr(object, table, expr, None, &[])
    }

    /// [`expr`](Self::expr) that also refuses a column `defined` lacks, when
    /// the table's columns are known, and the columns `restricted` names.
    pub(super) fn restricted_expr(
        &mut self,
        object: &'static str,
        table: TableRef,
        expr: &DdlExpr,
        defined: Option<&[ColumnDef]>,
        restricted: &[ExprRefusal],
    ) -> Result<(), RenderError> {
        let mode = DdlMode {
            object,
            table,
            scope: expr.scope,
            defined,
            restricted,
        };
        let sql = Writer::for_ddl(self.target, self.syntax, mode).ddl_expr(&expr.node)?;

        self.sql.push_str(&sql);

        Ok(())
    }

    /// Refuses a column of `object` whose handle was declared for a table
    /// other than `table`.
    pub(super) fn owned_columns<'a>(
        &self,
        object: &'static str,
        table: TableRef,
        columns: impl IntoIterator<Item = &'a IndexColumn>,
    ) -> Result<(), SchemaError> {
        for column in columns {
            match column.table {
                Some(recorded) if recorded != table => {
                    return Err(SchemaError::ForeignColumn {
                        object,
                        column: column.name,
                        recorded,
                        expected: table,
                    });
                }
                Some(_) | None => {}
            }
        }

        Ok(())
    }

    /// Writes the `WHERE` clause of a partial index: each condition checked
    /// against its own scope, parenthesized and ANDed when there are several.
    pub(super) fn predicates(&mut self, index: &Index) -> Result<(), RenderError> {
        let several = index.predicates.len() > 1;

        for (position, predicate) in index.predicates.iter().enumerate() {
            self.sql
                .push_str(if position == 0 { " WHERE " } else { " AND " });

            if several {
                self.sql.push('(');
            }

            self.expr(index.name, index.table, predicate)?;

            if several {
                self.sql.push(')');
            }
        }

        Ok(())
    }
}
