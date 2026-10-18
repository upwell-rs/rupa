//! Expressions: operator precedence and rendering.

use rupa_core::ir::{BinOp, ExprNode, UnOp};

use crate::RenderError;

use super::{MAX_LOWERING_DEPTH, Writer};

/// Binding strength, loosest first. An operand is parenthesized when its
/// precedence is below the minimum its context requires.
pub(super) mod prec {
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
pub(super) fn valid_function_name(name: &str) -> bool {
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
pub(super) fn valid_operator(op: &str) -> bool {
    let keyword = op
        .split(' ')
        .all(|w| !w.is_empty() && w.chars().all(|c| c.is_ascii_alphabetic()));
    let symbolic = !op.is_empty()
        && op.chars().all(|c| "+-*/<>=~!@#%^&|?".contains(c))
        && !op.contains("--")
        && !op.contains("/*");
    keyword || symbolic
}

impl Writer<'_> {
    pub(super) fn list(&mut self, items: &[ExprNode]) -> Result<(), RenderError> {
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
    ///
    /// `x IN ()` is invalid SQL; an empty list matches nothing.
    ///
    /// Render the operand on its own (parameters still number in
    /// order), then let the dialect wrap it.
    pub(super) fn expr(&mut self, node: &ExprNode, min: u8) -> Result<(), RenderError> {
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
            ExprNode::Column(c) => self.column(*c)?,
            ExprNode::Param(v) => self.param(v),
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
                if *op == BinOp::Like {
                    self.sql.push_str(self.syntax.like_escape());
                }
            }
            ExprNode::In { list, negated, .. } if list.is_empty() => {
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
                self.write_column(*column, &mut col)?;
                self.syntax.json_path(&col, path, *as_text, &mut self.sql)?;
            }
            ExprNode::Cast(e, ty) => {
                let outer = std::mem::take(&mut self.sql);
                let inner = self
                    .expr(e, prec::ANY)
                    .map(|()| std::mem::replace(&mut self.sql, outer))?;
                let cast = self.syntax.cast(&inner, *ty)?;
                self.sql.push_str(&cast);
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
