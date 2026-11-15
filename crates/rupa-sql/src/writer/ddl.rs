//! DDL mode: the writer rendering an expression for a schema object.
//!
//! Nothing is bound: values are written as checked literals, or refused. The
//! column set is the table's, so a column of another table is refused, and
//! calls DDL cannot hold (aggregates, window functions, well-known volatile
//! built-ins) are refused after DSL lowering, so a lowered call is checked
//! like a written one. The IR has no subquery node, so none can be written.

use rupa_core::column::ColumnMeta;
use rupa_core::ir::{ColumnRef, ExprNode, TableRef};
use rupa_core::{Dialect, Value};

use super::builtins::{self, Builtin};
use super::{Syntax, Writer, prec};
use crate::RenderError;
use crate::ddl::{ExprRefusal, SchemaError, literal};

/// What the writer knows about the schema object it renders for.
pub(crate) struct Mode<'a> {
    /// The object holding the expression, named in refusals.
    pub(crate) object: &'static str,
    /// The table the object belongs to: a column of a typed handle must be
    /// one of its.
    pub(crate) table: TableRef,
    /// The columns the expression may name, if known.
    pub(crate) scope: Option<&'a [ColumnMeta]>,
    /// Refusals for columns the dialect cannot use here, each naming its
    /// column.
    pub(crate) restricted: &'a [ExprRefusal],
}

impl<'a> Writer<'a> {
    /// A writer for one expression of a schema object.
    pub(crate) fn for_ddl(
        dialect: &'a dyn Dialect,
        syntax: &'a dyn Syntax,
        mode: Mode<'a>,
    ) -> Self {
        Self {
            ddl: Some(mode),
            ..Self::new(dialect, syntax)
        }
    }

    /// The SQL of `node`, with every value inline.
    pub(crate) fn ddl_expr(mut self, node: &ExprNode) -> Result<String, RenderError> {
        self.expr(node, prec::ANY)?;

        Ok(self.sql)
    }

    /// A value of the expression: a bound parameter, or in DDL mode a
    /// checked literal.
    pub(super) fn value(&mut self, value: &Value) -> Result<(), RenderError> {
        if self.ddl.is_none() {
            self.param(value);

            return Ok(());
        }

        if value.is_null() {
            self.sql.push_str("NULL");

            return Ok(());
        }

        let dialect = self.dialect.id();
        let cast = literal::bound_type(value, dialect);
        let mut literal = String::new();

        if literal::write(value, dialect, &mut literal).is_none() {
            return Err(self.refuse(ExprRefusal::Literal {
                ty: value.sql_type(),
            }));
        }

        if let Some(ty) = cast {
            self.sql.push_str("CAST(");
            self.sql.push_str(&literal);
            self.sql.push_str(" AS ");
            self.sql.push_str(ty);
            self.sql.push(')');
        } else {
            self.sql.push_str(&literal);
        }

        Ok(())
    }

    /// Refuses calls of the target dialect's built-ins that DDL cannot hold
    /// (see [`builtins::classify`]).
    pub(super) fn check_function(
        &self,
        name: &'static str,
        arguments: usize,
    ) -> Result<(), RenderError> {
        if self.ddl.is_none() {
            return Ok(());
        }

        let dialect = self.dialect.id();

        match builtins::classify(name, arguments, dialect) {
            Some(Builtin::Aggregate) => Err(self.refuse(ExprRefusal::Aggregate { function: name })),
            Some(Builtin::Window) => Err(self.refuse(ExprRefusal::Window { function: name })),
            Some(Builtin::Volatile) => Err(self.refuse(ExprRefusal::Volatile { function: name })),
            None => Ok(()),
        }
    }

    pub(super) fn check_column(&self, column: ColumnRef) -> Result<(), RenderError> {
        let Some(mode) = &self.ddl else {
            return Ok(());
        };

        let known = mode
            .scope
            .is_none_or(|columns| columns.iter().any(|c| c.name == column.name));
        let owned = column.table.is_none_or(|table| table == mode.table);

        if column.source.0 != 0 || !known || !owned {
            return Err(self.refuse(ExprRefusal::Column {
                column: column.name,
                table: column.table,
                expected: mode.table,
            }));
        }

        if let Some(reason) = mode
            .restricted
            .iter()
            .find(|reason| restricted_column(reason) == Some(column.name))
        {
            return Err(self.refuse(*reason));
        }

        Ok(())
    }

    fn refuse(&self, reason: ExprRefusal) -> RenderError {
        SchemaError::Expression {
            object: self.ddl.as_ref().map_or("", |mode| mode.object),
            dialect: self.dialect.id(),
            reason,
        }
        .into()
    }
}

fn restricted_column(reason: &ExprRefusal) -> Option<&'static str> {
    match reason {
        ExprRefusal::AutoIncrementColumn { column }
        | ExprRefusal::ReferentialActionColumn { column } => Some(column),
        _ => None,
    }
}
