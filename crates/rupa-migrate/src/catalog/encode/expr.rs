//! Canonical records for expressions of schema objects: the tree of the
//! expression IR, each node behind a tag naming its variant. Only what the
//! rendering depends on is recorded.

use rupa_core::ir::{BinOp, ColumnRef, ExprNode, PathSeg, UnOp};
use rupa_core::schema::{DdlExpr, Index, IndexPart};

use super::Encoder;
use super::kinds::sql_type_name;

impl Encoder<'_> {
    /// What an index adds to the record of its columns: where its
    /// expressions sit and what they are, then its predicate. A column-only,
    /// unfiltered index adds nothing, so it encodes as it did before
    /// expressions existed.
    pub(super) fn index_expressions(&mut self, index: &Index) {
        let expressions: Vec<_> = index
            .parts
            .iter()
            .enumerate()
            .filter_map(|(position, part)| match part {
                IndexPart::Expr(part) => Some((position, &part.expr)),
                IndexPart::Column(_) => None,
            })
            .collect();

        if !expressions.is_empty() {
            self.str("expression-parts");
            self.len(expressions.len());

            for (position, expr) in expressions {
                self.len(position);
                self.ddl_expr(expr);
            }
        }

        if let Some(predicate) = &index.predicate {
            self.str("predicate");
            self.ddl_expr(predicate);
        }
    }

    fn ddl_expr(&mut self, expr: &DdlExpr) {
        self.node(&expr.node);
    }

    fn node(&mut self, node: &ExprNode) {
        match node {
            ExprNode::Column(column) => {
                self.str("column");
                self.column_ref(*column);
            }
            ExprNode::Param(value) => {
                self.str("param");

                match value {
                    rupa_core::Value::Null(ty) => {
                        self.str("null");
                        self.str(sql_type_name(*ty));
                    }
                    value => self.value(value),
                }
            }
            ExprNode::Unary(op, operand) => {
                self.str("unary");
                self.str(match op {
                    UnOp::Not => "not",
                    UnOp::IsNull => "is-null",
                    UnOp::IsNotNull => "is-not-null",
                });
                self.node(operand);
            }
            ExprNode::Binary(op, lhs, rhs) => {
                self.str("binary");
                self.str(binary_name(*op));
                self.node(lhs);
                self.node(rhs);
            }
            ExprNode::In {
                expr,
                list,
                negated,
            } => {
                self.str("in");
                self.flag(*negated);
                self.node(expr);
                self.nodes(list);
            }
            ExprNode::JsonPath {
                column,
                path,
                as_text,
            } => {
                self.str("json-path");
                self.column_ref(*column);
                self.flag(*as_text);
                self.len(path.len());

                for segment in path {
                    match segment {
                        PathSeg::Key(key) => {
                            self.str("key");
                            self.str(key);
                        }
                        PathSeg::Index(index) => {
                            self.str("index");
                            self.len(*index as usize);
                        }
                    }
                }
            }
            ExprNode::Cast(operand, ty) => {
                self.str("cast");
                self.str(sql_type_name(*ty));
                self.node(operand);
            }
            ExprNode::Dsl(call) => {
                self.str("dsl");
                self.str(call.def.name);
                self.nodes(&call.args);
            }
            ExprNode::RawOp(op, lhs, rhs) => {
                self.str("raw-op");
                self.str(op);
                self.node(lhs);
                self.node(rhs);
            }
            ExprNode::Call(name, args) => {
                self.str("call");
                self.str(name);
                self.nodes(args);
            }
        }
    }

    fn nodes(&mut self, nodes: &[ExprNode]) {
        self.len(nodes.len());

        for node in nodes {
            self.node(node);
        }
    }

    fn column_ref(&mut self, column: ColumnRef) {
        self.len(usize::from(column.source.0));
        self.str(column.name);
    }
}

fn binary_name(op: BinOp) -> &'static str {
    match op {
        BinOp::Eq => "eq",
        BinOp::Ne => "ne",
        BinOp::Lt => "lt",
        BinOp::Le => "le",
        BinOp::Gt => "gt",
        BinOp::Ge => "ge",
        BinOp::And => "and",
        BinOp::Or => "or",
        BinOp::Like => "like",
    }
}
