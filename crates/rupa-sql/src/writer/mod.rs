//! Dialect-neutral SQL writer, parameterized by a [`Syntax`] for the parts
//! that differ between dialects.

#[cfg(test)]
mod tests;

mod expr;
mod syntax;

use rupa_core::Value;
use rupa_core::ir::{
    ColumnRef, Delete, Direction, ExprNode, FromClause, Insert, JoinKind, RawPart, Select,
    SourceId, Statement, TableRef, Update,
};
use rupa_core::{Capability, Dialect, SecurityContext};

use crate::{RenderError, Rendered};

use expr::prec;
pub(crate) use syntax::{NullsOrdering, Syntax, common_tx, isolation_sql};

/// Maximum nesting of DSL lowerings (a lowering may produce further DSL calls).
pub(crate) const MAX_LOWERING_DEPTH: usize = 32;

/// Renders one statement into SQL text and its bound parameters.
pub(crate) struct Writer<'a> {
    dialect: &'a dyn Dialect,
    syntax: &'a dyn Syntax,
    sql: String,
    params: Vec<Value>,
    /// When recording: the address of the IR value behind each placeholder,
    /// so the render cache can rebind fresh values to cached SQL.
    sources: Option<Vec<*const Value>>,
    lowering_depth: usize,
    /// Row sources of the statement being rendered; a column naming any
    /// other source is an error.
    source_count: usize,
    /// Whether columns are qualified by their source's alias (joins).
    qualify: bool,
}

impl<'a> Writer<'a> {
    pub(crate) fn new(dialect: &'a dyn Dialect, syntax: &'a dyn Syntax) -> Self {
        Self {
            dialect,
            syntax,
            sql: String::new(),
            params: Vec::new(),
            sources: None,
            lowering_depth: 0,
            source_count: 1,
            qualify: false,
        }
    }

    pub(crate) fn statement(self, stmt: &Statement) -> Result<Rendered, RenderError> {
        self.render(stmt).map(|(rendered, _)| rendered)
    }

    /// Renders `stmt`, also returning the address of the IR value behind each
    /// placeholder, in placeholder order (see [`crate::cache`]). A value the
    /// writer creates itself (none outside `ApplySecurity`) has an address
    /// that is not in the IR, which the cache treats as uncacheable.
    pub(crate) fn statement_with_sources(
        mut self,
        stmt: &Statement,
    ) -> Result<(Rendered, Vec<*const Value>), RenderError> {
        self.sources = Some(Vec::new());
        self.render(stmt)
            .map(|(rendered, sources)| (rendered, sources.unwrap_or_default()))
    }

    fn render(
        mut self,
        stmt: &Statement,
    ) -> Result<(Rendered, Option<Vec<*const Value>>), RenderError> {
        match stmt {
            Statement::Select(s) => self.select(s)?,
            Statement::Exists(s) => {
                self.sql.push_str("SELECT EXISTS (SELECT 1");
                self.select_tail(s)?;
                self.sql.push(')');
            }
            Statement::Insert(i) => self.insert(i)?,
            Statement::Update(u) => self.update(u)?,
            Statement::Delete(d) => self.delete(d)?,
            Statement::ApplySecurity(ctx) => self.apply_security(ctx)?,
            Statement::Raw(raw) => {
                for part in &raw.parts {
                    match part {
                        RawPart::Sql(s) => self.sql.push_str(s),
                        RawPart::Param(v) => self.param(v),
                    }
                }
            }
        }

        let rendered = Rendered {
            sql: self.sql,
            params: self.params,
        };
        Ok((rendered, self.sources))
    }

    fn select(&mut self, s: &Select) -> Result<(), RenderError> {
        self.enter_from(&s.from);
        self.sql.push_str("SELECT ");
        if s.projection.is_empty() {
            self.sql.push('1');
        }

        for (i, col) in s.projection.iter().enumerate() {
            if i > 0 {
                self.sql.push_str(", ");
            }

            self.column(*col)?;
        }

        self.select_tail(s)
    }

    /// `FROM ... [WHERE ...] [ORDER BY ...] [LIMIT ...] [OFFSET ...]`
    ///
    /// A condition may use this source and earlier ones only:
    /// a later alias isn't in scope yet.
    ///
    /// `(x IS NULL) ASC` puts NULLs last; DESC puts them first.
    fn select_tail(&mut self, s: &Select) -> Result<(), RenderError> {
        self.enter_from(&s.from);
        self.sql.push_str(" FROM ");
        match &s.from {
            FromClause::Table(t) => self.table(*t),
            FromClause::Join { base, joins } => {
                self.table(*base);
                self.alias(SourceId(0));
                for (n, join) in joins.iter().enumerate() {
                    self.sql.push_str(match join.kind {
                        JoinKind::Inner => " INNER JOIN ",
                        JoinKind::Left => " LEFT JOIN ",
                    });
                    self.table(join.table);
                    let id = u16::try_from(n + 1).map_err(|_| RenderError::TooManySources)?;
                    self.alias(SourceId(id));
                    self.sql.push_str(" ON ");
                    let all = std::mem::replace(&mut self.source_count, n + 2);
                    let on = self.expr(&join.on, prec::ANY);
                    self.source_count = all;
                    on?;
                }
            }
        }

        self.where_clause(s.filter.as_ref())?;
        for (i, o) in s.order_by.iter().enumerate() {
            self.sql.push_str(if i == 0 { " ORDER BY " } else { ", " });
            let dir = match o.direction {
                Direction::Asc => " ASC",
                Direction::Desc => " DESC",
            };
            match self.syntax.nulls_ordering() {
                NullsOrdering::Default => {
                    self.expr(&o.expr, prec::ANY)?;
                    self.sql.push_str(dir);
                }
                NullsOrdering::Keyword => {
                    self.expr(&o.expr, prec::ANY)?;
                    self.sql.push_str(dir);
                    self.sql.push_str(match o.direction {
                        Direction::Asc => " NULLS LAST",
                        Direction::Desc => " NULLS FIRST",
                    });
                }
                NullsOrdering::Emulate => {
                    self.sql.push('(');
                    self.expr(&o.expr, prec::ABOVE_CMP)?;
                    self.sql.push_str(" IS NULL)");
                    self.sql.push_str(dir);
                    self.sql.push_str(", ");
                    self.expr(&o.expr, prec::ANY)?;
                    self.sql.push_str(dir);
                }
            }
        }

        if let Some(limit) = &s.limit {
            self.sql.push_str(" LIMIT ");
            self.expr(limit, prec::ANY)?;
        } else if let (Some(_), Some(unbounded)) = (&s.offset, self.syntax.unbounded_limit()) {
            self.sql.push_str(" LIMIT ");
            self.sql.push_str(unbounded);
        }

        if let Some(offset) = &s.offset {
            self.sql.push_str(" OFFSET ");
            self.expr(offset, prec::ANY)?;
        }

        Ok(())
    }

    /// `SELECT set_config($1, $2, true), ..`: transaction-local settings, and
    /// the role through the `role` setting (`SET LOCAL ROLE`, but bindable).
    /// Every name and value is a bound parameter. Only Postgres has native
    /// row-level security (`NativeRls`); everything else is refused.
    fn apply_security(&mut self, ctx: &SecurityContext) -> Result<(), RenderError> {
        if !self.dialect.supports(Capability::NativeRls) {
            return Err(RenderError::UnsupportedCapability(Capability::NativeRls));
        }

        if let Some(bad) = ctx.invalid_key() {
            return Err(RenderError::InvalidSecurityKey(bad.to_owned()));
        }

        let mut pairs: Vec<(String, String)> =
            ctx.settings().map(|(k, v)| (k, v.to_owned())).collect();
        if let Some(role) = ctx.get_role() {
            pairs.push(("role".into(), role.to_owned()));
        }

        self.sql.push_str("SELECT ");
        for (i, (name, value)) in pairs.into_iter().enumerate() {
            if i > 0 {
                self.sql.push_str(", ");
            }

            self.sql.push_str("set_config(");
            self.param(&Value::Text(name));
            self.sql.push_str(", ");
            self.param(&Value::Text(value));
            self.sql.push_str(", true)");
        }

        Ok(())
    }

    fn insert(&mut self, i: &Insert) -> Result<(), RenderError> {
        if i.rows.is_empty() {
            return Err(RenderError::EmptyInsert);
        }

        self.sql.push_str("INSERT INTO ");
        self.table(i.table);
        self.sql.push_str(" (");
        for (n, c) in i.columns.iter().enumerate() {
            if n > 0 {
                self.sql.push_str(", ");
            }

            self.ident(c);
        }

        self.sql.push_str(") VALUES ");
        for (n, row) in i.rows.iter().enumerate() {
            if n > 0 {
                self.sql.push_str(", ");
            }

            self.list(row)?;
        }

        if let Some(cols) = &i.returning {
            if !self.dialect.supports(Capability::Returning) {
                return Err(RenderError::UnsupportedCapability(Capability::Returning));
            }

            self.sql.push_str(" RETURNING ");
            for (n, c) in cols.iter().enumerate() {
                if n > 0 {
                    self.sql.push_str(", ");
                }

                self.column(*c)?;
            }
        }

        Ok(())
    }

    fn update(&mut self, u: &Update) -> Result<(), RenderError> {
        if u.set.is_empty() {
            return Err(RenderError::EmptyUpdate);
        }

        self.sql.push_str("UPDATE ");
        self.table(u.table);
        self.sql.push_str(" SET ");
        for (n, (col, value)) in u.set.iter().enumerate() {
            if n > 0 {
                self.sql.push_str(", ");
            }

            self.ident(col);
            self.sql.push_str(" = ");
            self.expr(value, prec::ANY)?;
        }

        self.where_clause(u.filter.as_ref())
    }

    fn delete(&mut self, d: &Delete) -> Result<(), RenderError> {
        self.sql.push_str("DELETE FROM ");
        self.table(d.table);
        self.where_clause(d.filter.as_ref())
    }

    fn where_clause(&mut self, filter: Option<&ExprNode>) -> Result<(), RenderError> {
        if let Some(f) = filter {
            self.sql.push_str(" WHERE ");
            self.expr(f, prec::ANY)?;
        }

        Ok(())
    }

    fn table(&mut self, t: TableRef) {
        if let Some(schema) = t.schema {
            self.ident(schema);
            self.sql.push('.');
        }

        self.ident(t.name);
    }

    fn ident(&mut self, ident: &str) {
        self.syntax.quote_ident(ident, &mut self.sql);
    }

    /// Sets up column rendering for a select's sources. Every other statement
    /// has one source and unqualified columns.
    fn enter_from(&mut self, from: &FromClause) {
        self.source_count = from.source_count();
        self.qualify = matches!(from, FromClause::Join { .. });
    }

    /// ` AS "t<n>"`: aliases come from the source's position, never from data.
    fn alias(&mut self, id: SourceId) {
        self.sql.push_str(" AS ");
        self.ident(&format!("t{}", id.0));
    }

    fn column(&mut self, c: ColumnRef) -> Result<(), RenderError> {
        let mut out = std::mem::take(&mut self.sql);
        let r = self.write_column(c, &mut out);
        self.sql = out;
        r
    }

    /// Writes `c`, qualified by its source's alias in a join.
    fn write_column(&self, c: ColumnRef, out: &mut String) -> Result<(), RenderError> {
        if usize::from(c.source.0) >= self.source_count {
            return Err(RenderError::UnknownSource(c.source));
        }

        if self.qualify {
            self.syntax.quote_ident(&format!("t{}", c.source.0), out);
            out.push('.');
        }

        self.syntax.quote_ident(c.name, out);
        Ok(())
    }

    fn param(&mut self, v: &Value) {
        if let Some(sources) = &mut self.sources {
            sources.push(std::ptr::from_ref(v));
        }

        self.params.push(v.clone());
        self.syntax.placeholder(self.params.len(), &mut self.sql);
    }
}
