//! `CREATE INDEX`: rendering and the dialect checks on kind and columns.

use rupa_core::DialectId;
use rupa_core::ir::TableRef;
use rupa_core::schema::{Index, IndexSupport};

use super::{Ddl, IndexRefusal, SchemaError, types};

impl Ddl {
    pub(super) fn create_index(&mut self, index: &Index) -> Result<(), SchemaError> {
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

        for (i, column) in index.columns.iter().enumerate() {
            if i > 0 {
                self.sql.push_str(", ");
            }

            self.ident(column.name);
        }

        self.sql.push(')');

        Ok(())
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

        if index.columns.is_empty() {
            return Err(SchemaError::EmptyIndex(index.name));
        }

        if index.unique && !kind.unique {
            return Err(refuse(IndexRefusal::Unique { kind: kind.name }));
        }

        if index.columns.len() > 1 && !kind.multi_column {
            return Err(refuse(IndexRefusal::MultiColumn { kind: kind.name }));
        }

        if self.dialect == DialectId::MySql
            && let Some(column) = index.columns.iter().find(|c| types::mysql_unbounded(c.ty))
        {
            return Err(refuse(IndexRefusal::Unbounded {
                column: column.name,
                ty: column.ty,
            }));
        }

        Ok(using)
    }
}
