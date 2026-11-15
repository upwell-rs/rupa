//! Typed statement builders. Each produces a dialect-free [`Query<R>`]; no IO.

mod delete;
mod insert;
mod raw;
mod select;
mod update;

use crate::ir::{BinOp, ColumnRef, ExprNode};
use crate::value::Value;

pub use delete::{DeleteBuilder, delete};
pub use insert::{InsertBuilder, InsertRows, insert};
pub use raw::{RawBuilder, raw};
pub use select::{SelectBuilder, get, select};
pub use update::{UpdateBuilder, update};

pub(crate) fn and_filter(filter: &mut Option<ExprNode>, cond: ExprNode) {
    *filter = Some(match filter.take() {
        None => cond,
        Some(prev) => ExprNode::Binary(BinOp::And, Box::new(prev), Box::new(cond)),
    });
}

/// SQL LIMIT/OFFSET are signed 64-bit; anything larger means "no bound".
pub(crate) fn count_param(n: u64) -> ExprNode {
    ExprNode::Param(Value::I64(i64::try_from(n).unwrap_or(i64::MAX)))
}

/// `id_col_1 = $1 AND id_col_2 = $2 ...`
pub(crate) fn key_filter(columns: &[&'static str], values: Vec<Value>) -> ExprNode {
    debug_assert_eq!(
        columns.len(),
        values.len(),
        "key arity must match ID_COLUMNS"
    );
    columns
        .iter()
        .zip(values)
        .map(|(c, v)| {
            ExprNode::Binary(
                BinOp::Eq,
                Box::new(ExprNode::Column(ColumnRef::new(c))),
                Box::new(ExprNode::Param(v)),
            )
        })
        .reduce(|a, b| ExprNode::Binary(BinOp::And, Box::new(a), Box::new(b)))
        .expect("an entity has at least one id column")
}
