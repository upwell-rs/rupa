//! Expressions of schema objects, shared by everything that holds one
//! (index parts and predicates, and constraints).

use rupa_core::ir::TableRef;
use rupa_core::schema::{DdlExpr, Index};

use super::{Ddl, ExprRefusal};
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
        self.restricted_expr(object, table, expr, &[])
    }

    /// [`expr`](Self::expr) that also refuses the columns `restricted` names.
    pub(super) fn restricted_expr(
        &mut self,
        object: &'static str,
        table: TableRef,
        expr: &DdlExpr,
        restricted: &[ExprRefusal],
    ) -> Result<(), RenderError> {
        let mode = DdlMode {
            object,
            table,
            scope: expr.scope,
            restricted,
        };
        let sql = Writer::for_ddl(self.target, self.syntax, mode).ddl_expr(&expr.node)?;

        self.sql.push_str(&sql);

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
