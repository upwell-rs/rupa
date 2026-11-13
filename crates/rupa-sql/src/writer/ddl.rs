//! DDL mode: the writer rendering an expression for a schema object.
//!
//! Nothing is bound: values are written as checked literals, or refused. The
//! column set is the table's, so a column of another table is refused, and
//! calls DDL cannot hold (aggregates, window functions, well-known volatile
//! built-ins) are refused after DSL lowering, so a lowered call is checked
//! like a written one. The IR has no subquery node, so none can be written.

use rupa_core::column::ColumnMeta;
use rupa_core::ir::{ColumnRef, ExprNode, TableRef};
use rupa_core::{Dialect, DialectId, Value};

use super::{Syntax, Writer, prec};
use crate::RenderError;
use crate::ddl::{ExprRefusal, SchemaError, literal};

const AGGREGATES: &[&str] = &[
    "array_agg",
    "avg",
    "bit_and",
    "bit_or",
    "bit_xor",
    "bool_and",
    "bool_or",
    "count",
    "every",
    "group_concat",
    "json_agg",
    "json_arrayagg",
    "json_group_array",
    "json_group_object",
    "json_objectagg",
    "jsonb_agg",
    "max",
    "min",
    "string_agg",
    "sum",
    "total",
];

const WINDOWS: &[&str] = &[
    "cume_dist",
    "dense_rank",
    "first_value",
    "lag",
    "last_value",
    "lead",
    "nth_value",
    "ntile",
    "percent_rank",
    "rank",
    "row_number",
];

const VOLATILE: &[&str] = &[
    "changes",
    "clock_timestamp",
    "connection_id",
    "curdate",
    "current_date",
    "current_time",
    "current_timestamp",
    "current_user",
    "curtime",
    "currval",
    "database",
    "found_rows",
    "gen_random_uuid",
    "last_insert_id",
    "last_insert_rowid",
    "lastval",
    "localtime",
    "localtimestamp",
    "nextval",
    "now",
    "rand",
    "random",
    "randomblob",
    "row_count",
    "session_user",
    "setval",
    "statement_timestamp",
    "sysdate",
    "timeofday",
    "total_changes",
    "transaction_timestamp",
    "user",
    "utc_date",
    "utc_time",
    "utc_timestamp",
    "uuid",
    "uuid_generate_v4",
    "uuid_short",
    "version",
];

/// What the writer knows about the schema object it renders for.
pub(crate) struct Mode<'a> {
    /// The object holding the expression, named in refusals.
    pub(crate) object: &'static str,
    /// The table the object belongs to: a column of a typed handle must be
    /// one of its.
    pub(crate) table: TableRef,
    /// The columns the expression may name, if known.
    pub(crate) scope: Option<&'a [ColumnMeta]>,
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

        if literal::write(value, dialect, &mut self.sql).is_none() {
            return Err(self.refuse(ExprRefusal::Literal {
                ty: value.sql_type(),
            }));
        }

        Ok(())
    }

    /// Refuses calls DDL cannot hold. SQLite's `max` and `min` with several
    /// arguments are scalar functions and allowed; with one they aggregate.
    /// PostgreSQL and MySQL have no scalar `max(a, b)` (they spell it
    /// `greatest`), so there the name alone is refused as an aggregate.
    pub(super) fn check_function(
        &self,
        name: &'static str,
        arguments: usize,
    ) -> Result<(), RenderError> {
        if self.ddl.is_none() {
            return Ok(());
        }

        let bare = name.rsplit('.').next().unwrap_or(name);
        let named = |list: &[&str]| list.iter().any(|f| f.eq_ignore_ascii_case(bare));
        let scalar = self.dialect.id() == DialectId::Sqlite
            && arguments > 1
            && (bare.eq_ignore_ascii_case("max") || bare.eq_ignore_ascii_case("min"));

        if named(AGGREGATES) && !scalar {
            return Err(self.refuse(ExprRefusal::Aggregate { function: name }));
        }

        if named(WINDOWS) {
            return Err(self.refuse(ExprRefusal::Window { function: name }));
        }

        if named(VOLATILE) {
            return Err(self.refuse(ExprRefusal::Volatile { function: name }));
        }

        Ok(())
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
