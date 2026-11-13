//! `CREATE INDEX`: rendering and the dialect checks on kind and columns.

use rupa_core::DialectId;
use rupa_core::ir::{ExprNode, TableRef};
use rupa_core::schema::{Index, IndexPart, IndexSupport};

use super::{Ddl, IndexRefusal, SchemaError, types};
use crate::RenderError;
use crate::writer::MAX_LOWERING_DEPTH;

impl Ddl<'_> {
    pub(super) fn create_index(&mut self, index: &Index) -> Result<(), RenderError> {
        let using = self.index_method(index)?;

        self.sql.push_str(if index.unique {
            "CREATE UNIQUE INDEX "
        } else {
            "CREATE INDEX "
        });

        if self.dialect == DialectId::Sqlite {
            self.table(TableRef {
                name: index.name,
                ..index.table
            });
            self.sql.push_str(" ON ");
            self.ident(index.table.name);
        } else {
            self.ident(index.name);

            if self.dialect == DialectId::MySql {
                self.using(using);
            }

            self.sql.push_str(" ON ");
            self.table(index.table);
        }

        if self.dialect == DialectId::Postgres {
            self.using(using);
        }

        self.sql.push_str(" (");

        for (i, part) in index.parts.iter().enumerate() {
            if i > 0 {
                self.sql.push_str(", ");
            }

            match part {
                IndexPart::Column(column) => self.ident(column.name),
                IndexPart::Expr(part) if self.bare_column(&part.expr.node, 0) => {
                    self.expr(index.name, index.table, &part.expr)?;
                }
                IndexPart::Expr(part) => {
                    self.sql.push('(');
                    self.expr(index.name, index.table, &part.expr)?;
                    self.sql.push(')');
                }
            }
        }

        self.sql.push(')');

        if let Some(predicate) = &index.predicate {
            self.sql.push_str(" WHERE ");
            self.expr(index.name, index.table, predicate)?;
        }

        Ok(())
    }

    /// Whether `node`, lowered for the dialect, is a single column. Such a
    /// part is written as a plain key part: MySQL refuses a functional key
    /// part that is only a column (error 3762), and every dialect indexes
    /// both the same way. Lowering is followed as deep as the writer follows
    /// it.
    fn bare_column(&self, node: &ExprNode, depth: usize) -> bool {
        match node {
            ExprNode::Column(_) => true,
            ExprNode::Dsl(call) if depth < MAX_LOWERING_DEPTH => call
                .lower(self.target)
                .is_ok_and(|lowered| self.bare_column(&lowered, depth + 1)),
            _ => false,
        }
    }

    fn using(&mut self, method: Option<&'static str>) {
        if let Some(method) = method {
            self.sql.push_str(" USING ");
            self.sql.push_str(method);
        }
    }

    /// Checks `index` against the dialect and returns its `USING` method.
    fn index_method(&self, index: &Index) -> Result<Option<&'static str>, SchemaError> {
        let kind = index.kind;
        let refuse = |reason| SchemaError::Index {
            index: index.name,
            dialect: self.dialect,
            reason,
        };
        let using = match kind.support(self.dialect) {
            IndexSupport::Default => None,
            IndexSupport::Using(_) if self.dialect == DialectId::Sqlite => {
                return Err(refuse(IndexRefusal::Kind {
                    kind: kind.name,
                    reason: "SQLite has no index methods",
                }));
            }
            IndexSupport::Using(method) => Some(method),
            IndexSupport::Unsupported(reason) => {
                return Err(refuse(IndexRefusal::Kind {
                    kind: kind.name,
                    reason,
                }));
            }
        };

        if index.parts.is_empty() {
            return Err(SchemaError::EmptyIndex(index.name));
        }

        if index.unique && !kind.unique {
            return Err(refuse(IndexRefusal::Unique { kind: kind.name }));
        }

        if index.parts.len() > 1 && !kind.multi_column {
            return Err(refuse(IndexRefusal::MultiColumn { kind: kind.name }));
        }

        if self.dialect == DialectId::MySql {
            if index.predicate.is_some() {
                return Err(refuse(IndexRefusal::Partial));
            }

            for part in &index.parts {
                match part {
                    IndexPart::Column(column) if types::mysql_unbounded(column.ty) => {
                        return Err(refuse(IndexRefusal::Unbounded {
                            column: column.name,
                            ty: column.ty,
                        }));
                    }
                    IndexPart::Expr(part) if types::mysql_unbounded(part.ty) => {
                        return Err(refuse(IndexRefusal::UnboundedExpr { ty: part.ty }));
                    }
                    IndexPart::Column(_) | IndexPart::Expr(_) => {}
                }
            }
        }

        Ok(using)
    }
}
