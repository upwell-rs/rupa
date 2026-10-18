//! Lowering DSL calls before a statement is shaped.

use rupa_core::dialect::Dialect;
use rupa_core::ir::{ExprNode, FromClause, Statement};

use super::MAX_LOWERING_DEPTH;
use crate::RenderError;

/// `statement` with every DSL call lowered for `dialect`, recursively.
pub(super) fn lower_statement(
    statement: &Statement,
    dialect: &dyn Dialect,
) -> Result<Statement, RenderError> {
    let mut statement = statement.clone();
    let lower = |e: &mut ExprNode| lower_expr(e, dialect, 0);
    let lower_opt = |e: &mut Option<ExprNode>| e.as_mut().map_or(Ok(()), lower);
    match &mut statement {
        Statement::Select(s) | Statement::Exists(s) => {
            if let FromClause::Join { joins, .. } = &mut s.from {
                for join in joins {
                    lower(&mut join.on)?;
                }
            }

            lower_opt(&mut s.filter)?;
            lower_opt(&mut s.limit)?;
            lower_opt(&mut s.offset)?;
            for o in &mut s.order_by {
                lower(&mut o.expr)?;
            }
        }
        Statement::Insert(i) => {
            for e in i.rows.iter_mut().flatten() {
                lower(e)?;
            }
        }
        Statement::Update(u) => {
            for (_, e) in &mut u.set {
                lower(e)?;
            }

            lower_opt(&mut u.filter)?;
        }
        Statement::Delete(d) => lower_opt(&mut d.filter)?,
        Statement::Raw(_) | Statement::ApplySecurity(_) => {}
    }

    Ok(statement)
}

fn lower_expr(e: &mut ExprNode, dialect: &dyn Dialect, depth: usize) -> Result<(), RenderError> {
    match e {
        ExprNode::Dsl(call) => {
            if depth >= MAX_LOWERING_DEPTH {
                return Err(RenderError::TooDeep);
            }

            let mut lowered = call.lower(dialect)?;
            lower_expr(&mut lowered, dialect, depth + 1)?;
            *e = lowered;
        }
        ExprNode::Column(_) | ExprNode::Param(_) | ExprNode::JsonPath { .. } => {}
        ExprNode::Unary(_, inner) | ExprNode::Cast(inner, _) => lower_expr(inner, dialect, depth)?,
        ExprNode::Binary(_, l, r) | ExprNode::RawOp(_, l, r) => {
            lower_expr(l, dialect, depth)?;
            lower_expr(r, dialect, depth)?;
        }
        ExprNode::In { expr, list, .. } => {
            lower_expr(expr, dialect, depth)?;
            for item in list {
                lower_expr(item, dialect, depth)?;
            }
        }
        ExprNode::Call(_, args) => {
            for arg in args {
                lower_expr(arg, dialect, depth)?;
            }
        }
    }

    Ok(())
}
