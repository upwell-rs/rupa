//! Dialect-neutral SQL writer, parameterized by a [`Syntax`] for the parts
//! that differ between dialects.

use rupa_core::Dialect;
use rupa_core::ir::{
    BinOp, ColumnRef, Delete, Direction, ExprNode, FromClause, Insert, PathSeg, RawPart, Select,
    Statement, TableRef, UnOp, Update,
};
use rupa_core::{SqlType, Value};

use crate::{RenderError, Rendered};

/// Maximum nesting of DSL lowerings (a lowering may produce further DSL calls).
const MAX_LOWERING_DEPTH: usize = 32;

pub(crate) trait Syntax {
    /// Writes the placeholder for 1-based parameter `n`.
    fn placeholder(&self, n: usize, out: &mut String);

    fn quote_ident(&self, ident: &str, out: &mut String) {
        out.push('"');
        for c in ident.chars() {
            if c == '"' {
                out.push('"');
            }
            out.push(c);
        }
        out.push('"');
    }

    fn cast_type(&self, ty: SqlType) -> &'static str;

    /// Writes a path into the already-rendered JSON column `column`.
    fn json_path(
        &self,
        column: &str,
        path: &[PathSeg],
        as_text: bool,
        out: &mut String,
    ) -> Result<(), RenderError>;
}

pub(crate) struct Writer<'a> {
    dialect: &'a dyn Dialect,
    syntax: &'a dyn Syntax,
    sql: String,
    params: Vec<Value>,
    lowering_depth: usize,
}

/// Binding strength, loosest first. An operand is parenthesized when its
/// precedence is below the minimum its context requires.
mod prec {
    pub const ANY: u8 = 0;
    pub const OR: u8 = 1;
    pub const AND: u8 = 2;
    pub const NOT: u8 = 3;
    pub const CMP: u8 = 4;
    /// Operands of comparisons and NOT: anything looser than an atom-like
    /// expression is wrapped (comparisons don't chain).
    pub const ABOVE_CMP: u8 = 5;
    /// JSON path operators (`->`, `->>`): bind tighter than comparisons and
    /// `IS`, but share a level with other symbolic operators such as `@>`.
    pub const JSON_OP: u8 = 9;
    pub const ATOM: u8 = 10;
}

fn precedence(node: &ExprNode) -> u8 {
    match node {
        ExprNode::Binary(BinOp::Or, ..) => prec::OR,
        ExprNode::Binary(BinOp::And, ..) => prec::AND,
        ExprNode::Unary(UnOp::Not, _) => prec::NOT,
        ExprNode::In { list, .. } if list.is_empty() => prec::ATOM,
        ExprNode::Binary(..) | ExprNode::Unary(..) | ExprNode::In { .. } | ExprNode::RawOp(..) => {
            prec::CMP
        }
        ExprNode::JsonPath { .. } => prec::JSON_OP,
        ExprNode::Column(_)
        | ExprNode::Param(_)
        | ExprNode::Cast(..)
        | ExprNode::Call(..)
        | ExprNode::Dsl(_) => prec::ATOM,
    }
}

fn binop(op: BinOp) -> &'static str {
    match op {
        BinOp::Eq => "=",
        BinOp::Ne => "<>",
        BinOp::Lt => "<",
        BinOp::Le => "<=",
        BinOp::Gt => ">",
        BinOp::Ge => ">=",
        BinOp::And => "AND",
        BinOp::Or => "OR",
        BinOp::Like => "LIKE",
    }
}

fn binop_prec(op: BinOp) -> u8 {
    match op {
        BinOp::Or => prec::OR,
        BinOp::And => prec::AND,
        _ => prec::ABOVE_CMP,
    }
}

/// `name` or `schema.name`, each part an identifier.
fn valid_function_name(name: &str) -> bool {
    !name.is_empty()
        && name.split('.').all(|part| {
            let mut chars = part.chars();
            chars
                .next()
                .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        })
}

/// Either a keyword operator (`ILIKE`, `IS DISTINCT FROM`) or a symbolic one
/// (`@>`, `->>`), never containing comment starters.
fn valid_operator(op: &str) -> bool {
    let keyword = op
        .split(' ')
        .all(|w| !w.is_empty() && w.chars().all(|c| c.is_ascii_alphabetic()));
    let symbolic = !op.is_empty()
        && op.chars().all(|c| "+-*/<>=~!@#%^&|?".contains(c))
        && !op.contains("--")
        && !op.contains("/*");
    keyword || symbolic
}

impl<'a> Writer<'a> {
    pub(crate) fn new(dialect: &'a dyn Dialect, syntax: &'a dyn Syntax) -> Self {
        Self {
            dialect,
            syntax,
            sql: String::new(),
            params: Vec::new(),
            lowering_depth: 0,
        }
    }

    pub(crate) fn statement(mut self, stmt: &Statement) -> Result<Rendered, RenderError> {
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
            Statement::Raw(raw) => {
                for part in &raw.parts {
                    match part {
                        RawPart::Sql(s) => self.sql.push_str(s),
                        RawPart::Param(v) => self.param(v.clone()),
                    }
                }
            }
        }
        Ok(Rendered {
            sql: self.sql,
            params: self.params,
        })
    }

    fn select(&mut self, s: &Select) -> Result<(), RenderError> {
        self.sql.push_str("SELECT ");
        if s.projection.is_empty() {
            self.sql.push('1');
        }
        for (i, col) in s.projection.iter().enumerate() {
            if i > 0 {
                self.sql.push_str(", ");
            }
            self.column(*col);
        }
        self.select_tail(s)
    }

    /// `FROM ... [WHERE ...] [ORDER BY ...] [LIMIT ...] [OFFSET ...]`
    fn select_tail(&mut self, s: &Select) -> Result<(), RenderError> {
        self.sql.push_str(" FROM ");
        match &s.from {
            FromClause::Table(t) => self.table(*t),
        }
        self.where_clause(s.filter.as_ref())?;
        for (i, o) in s.order_by.iter().enumerate() {
            self.sql.push_str(if i == 0 { " ORDER BY " } else { ", " });
            self.expr(&o.expr, prec::ANY)?;
            self.sql.push_str(match o.direction {
                Direction::Asc => " ASC",
                Direction::Desc => " DESC",
            });
        }
        if let Some(limit) = &s.limit {
            self.sql.push_str(" LIMIT ");
            self.expr(limit, prec::ANY)?;
        }
        if let Some(offset) = &s.offset {
            self.sql.push_str(" OFFSET ");
            self.expr(offset, prec::ANY)?;
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

    /// Single-source statements render unqualified columns; qualification by
    /// `SourceId` arrives with joins.
    fn column(&mut self, c: ColumnRef) {
        self.ident(c.name);
    }

    fn param(&mut self, v: Value) {
        self.params.push(v);
        self.syntax.placeholder(self.params.len(), &mut self.sql);
    }

    fn list(&mut self, items: &[ExprNode]) -> Result<(), RenderError> {
        self.sql.push('(');
        for (n, e) in items.iter().enumerate() {
            if n > 0 {
                self.sql.push_str(", ");
            }
            self.expr(e, prec::ANY)?;
        }
        self.sql.push(')');
        Ok(())
    }

    /// Renders `node`, parenthesized if it binds looser than `min`.
    fn expr(&mut self, node: &ExprNode, min: u8) -> Result<(), RenderError> {
        if let ExprNode::Dsl(call) = node {
            if self.lowering_depth >= MAX_LOWERING_DEPTH {
                return Err(RenderError::TooDeep);
            }
            let lowered = call.lower(self.dialect)?;
            self.lowering_depth += 1;
            let r = self.expr(&lowered, min);
            self.lowering_depth -= 1;
            return r;
        }

        let wrap = precedence(node) < min;
        if wrap {
            self.sql.push('(');
        }
        match node {
            ExprNode::Column(c) => self.column(*c),
            ExprNode::Param(v) => self.param(v.clone()),
            ExprNode::Unary(UnOp::Not, e) => {
                self.sql.push_str("NOT ");
                self.expr(e, prec::ABOVE_CMP)?;
            }
            ExprNode::Unary(op @ (UnOp::IsNull | UnOp::IsNotNull), e) => {
                self.expr(e, prec::ABOVE_CMP)?;
                self.sql.push_str(if *op == UnOp::IsNull {
                    " IS NULL"
                } else {
                    " IS NOT NULL"
                });
            }
            ExprNode::Binary(op, l, r) => {
                let p = binop_prec(*op);
                self.expr(l, p)?;
                self.sql.push(' ');
                self.sql.push_str(binop(*op));
                self.sql.push(' ');
                self.expr(r, p)?;
            }
            ExprNode::In { list, negated, .. } if list.is_empty() => {
                // `x IN ()` is invalid SQL; an empty list matches nothing.
                self.sql.push_str(if *negated { "TRUE" } else { "FALSE" });
            }
            ExprNode::In {
                expr,
                list,
                negated,
            } => {
                self.expr(expr, prec::ABOVE_CMP)?;
                self.sql
                    .push_str(if *negated { " NOT IN " } else { " IN " });
                self.list(list)?;
            }
            ExprNode::JsonPath {
                column,
                path,
                as_text,
            } => {
                let mut col = String::new();
                self.syntax.quote_ident(column.name, &mut col);
                self.syntax.json_path(&col, path, *as_text, &mut self.sql)?;
            }
            ExprNode::Cast(e, ty) => {
                self.sql.push_str("CAST(");
                self.expr(e, prec::ANY)?;
                self.sql.push_str(" AS ");
                self.sql.push_str(self.syntax.cast_type(*ty));
                self.sql.push(')');
            }
            ExprNode::RawOp(op, l, r) => {
                if !valid_operator(op) {
                    return Err(RenderError::InvalidToken(op));
                }
                self.expr(l, prec::ATOM)?;
                self.sql.push(' ');
                self.sql.push_str(op);
                self.sql.push(' ');
                self.expr(r, prec::ATOM)?;
            }
            ExprNode::Call(name, args) => {
                if !valid_function_name(name) {
                    return Err(RenderError::InvalidToken(name));
                }
                self.sql.push_str(name);
                self.list(args)?;
            }
            ExprNode::Dsl(_) => unreachable!("lowered above"),
        }
        if wrap {
            self.sql.push(')');
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operator_validation() {
        for ok in [
            "ILIKE",
            "NOT ILIKE",
            "IS DISTINCT FROM",
            "@>",
            "->>",
            "?|",
            "<->",
        ] {
            assert!(valid_operator(ok), "{ok}");
        }
        for bad in [
            "", " ILIKE", "ILIKE ", "a;b", "--", "@>--", "/*", "'", "x1", "a  b",
        ] {
            assert!(!valid_operator(bad), "{bad}");
        }
    }

    #[test]
    fn function_name_validation() {
        for ok in ["lower", "LOWER", "pg_catalog.lower", "_f1"] {
            assert!(valid_function_name(ok), "{ok}");
        }
        for bad in ["", "1f", "a.", ".a", "lower()", "a b", "a;drop"] {
            assert!(!valid_function_name(bad), "{bad}");
        }
    }
}
