//! Expressions of schema objects, shared by everything that holds one
//! (index parts and predicates, and constraints).

use rupa_core::ir::TableRef;
use rupa_core::schema::DdlExpr;

use super::Ddl;
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
        let mode = DdlMode {
            object,
            table,
            scope: expr.scope,
        };
        let sql = Writer::for_ddl(self.target, self.syntax, mode).ddl_expr(&expr.node)?;

        self.sql.push_str(&sql);

        Ok(())
    }
}
