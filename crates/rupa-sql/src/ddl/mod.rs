//! Renders [`rupa_core::schema`] operations to one DDL statement each.
//!
//! What a dialect cannot express is a [`SchemaError`], never a silent
//! rewrite: SQLite cannot add a `NOT NULL` column without a default, MySQL
//! cannot key unbounded text, and identity columns follow each dialect's
//! rules. Operations that need a table rebuild on SQLite are not offered.
//!
//! Checks that need column types or names are made where the operation
//! carries them, in `CREATE TABLE` and `ADD COLUMN`. An index or a foreign
//! key into an existing table names columns without types, so a MySQL index
//! over unbounded text or an unknown column is rejected by the server.

mod error;
mod types;

use rupa_core::ir::TableRef;
use rupa_core::schema::{ColumnDef, ForeignKey, Index, ReferentialAction, SchemaOp, Table};
use rupa_core::{Dialect, DialectId, Generated};

pub use error::SchemaError;

use crate::writer::Syntax;
use crate::{RenderError, syntax_for};

/// Renders `op` for `dialect` as one statement, without a trailing `;`.
pub fn render_schema(op: &SchemaOp, dialect: &dyn Dialect) -> Result<String, RenderError> {
    let mut ddl = Ddl {
        dialect: dialect.id(),
        syntax: syntax_for(dialect)?,
        sql: String::new(),
    };

    ddl.op(op)?;

    Ok(ddl.sql)
}

/// One statement being written.
struct Ddl {
    dialect: DialectId,
    syntax: &'static dyn Syntax,
    sql: String,
}

impl Ddl {
    fn op(&mut self, op: &SchemaOp) -> Result<(), SchemaError> {
        match op {
            SchemaOp::CreateTable(table) => self.create_table(table)?,
            SchemaOp::DropTable(table) => {
                self.sql.push_str("DROP TABLE ");
                self.table(*table);
            }
            SchemaOp::RenameTable { table, to } => {
                self.alter(*table);
                self.sql.push_str(" RENAME TO ");

                if self.dialect == DialectId::MySql {
                    self.table(TableRef { name: to, ..*table });
                } else {
                    self.ident(to);
                }
            }
            SchemaOp::AddColumn { table, column } => {
                self.addable(column)?;
                self.alter(*table);
                self.sql.push_str(" ADD COLUMN ");
                self.column(column, false)?;
            }
            SchemaOp::DropColumn { table, column } => {
                self.alter(*table);
                self.sql.push_str(" DROP COLUMN ");
                self.ident(column);
            }
            SchemaOp::RenameColumn { table, from, to } => {
                self.alter(*table);
                self.sql.push_str(" RENAME COLUMN ");
                self.ident(from);
                self.sql.push_str(" TO ");
                self.ident(to);
            }
            SchemaOp::CreateIndex(index) => self.create_index(index)?,
            SchemaOp::DropIndex { table, name } => {
                self.sql.push_str("DROP INDEX ");

                if self.dialect == DialectId::MySql {
                    self.ident(name);
                    self.sql.push_str(" ON ");
                    self.table(*table);
                } else {
                    self.table(TableRef { name, ..*table });
                }
            }
        }

        Ok(())
    }

    fn create_table(&mut self, table: &Table) -> Result<(), SchemaError> {
        let name = table.table.name;
        let identity = self.identity(table)?;
        let sqlite_identity = self.dialect == DialectId::Sqlite && identity.is_some();

        if table.columns.is_empty() {
            return Err(SchemaError::NoColumns(name));
        }

        for column in table
            .primary_key
            .iter()
            .chain(table.foreign_keys.iter().flat_map(|f| &f.columns))
        {
            self.find(table, column)?;
        }

        self.sql.push_str("CREATE TABLE ");
        self.table(table.table);
        self.sql.push_str(" (");

        for (i, column) in table.columns.iter().enumerate() {
            if i > 0 {
                self.sql.push_str(", ");
            }

            self.column(column, sqlite_identity && identity == Some(column.name))?;
        }

        if !table.primary_key.is_empty() && !sqlite_identity {
            self.keyable(table, &table.primary_key)?;
            self.sql.push_str(", PRIMARY KEY (");
            self.idents(&table.primary_key);
            self.sql.push(')');
        }

        for foreign_key in &table.foreign_keys {
            self.foreign_key(&table.table, foreign_key)?;
        }

        self.sql.push(')');

        Ok(())
    }

    /// The table's identity column, checked against the dialect's rules.
    fn identity(&self, table: &Table) -> Result<Option<&'static str>, SchemaError> {
        let mut identities = table
            .columns
            .iter()
            .filter(|c| c.generated == Some(Generated::Identity));
        let Some(identity) = identities.next() else {
            return Ok(None);
        };

        if identities.next().is_some() {
            return Err(SchemaError::SeveralIdentities(table.table.name));
        }

        if self.dialect != DialectId::Postgres && table.primary_key != [identity.name] {
            return Err(SchemaError::IdentityKey(identity.name));
        }

        Ok(Some(identity.name))
    }

    /// `name TYPE [NOT NULL] [DEFAULT ..] [UNIQUE]`, or SQLite's rowid alias
    /// for its identity key.
    fn column(&mut self, column: &ColumnDef, sqlite_identity: bool) -> Result<(), SchemaError> {
        let identity = column.generated == Some(Generated::Identity);

        if identity && (!types::is_integer(column.ty) || column.default.is_some()) {
            return Err(SchemaError::IdentityType(column.name));
        }

        if column.generated == Some(Generated::Database) && column.default.is_none() {
            return Err(SchemaError::MissingDefault(column.name));
        }

        if column.unique && self.dialect == DialectId::MySql && types::mysql_unbounded(column.ty) {
            return Err(self.unindexable(column));
        }

        self.ident(column.name);

        if sqlite_identity {
            self.sql.push_str(" INTEGER PRIMARY KEY AUTOINCREMENT");

            return Ok(());
        }

        self.sql.push(' ');
        self.sql.push_str(&types::type_name(column, self.dialect)?);

        if identity && self.dialect == DialectId::Postgres {
            self.sql.push_str(" GENERATED BY DEFAULT AS IDENTITY");
        }

        if !column.nullable || identity {
            self.sql.push_str(" NOT NULL");
        }

        if identity && self.dialect == DialectId::MySql {
            self.sql.push_str(" AUTO_INCREMENT");
        }

        if let Some(default) = &column.default {
            self.sql.push_str(" DEFAULT ");
            self.sql
                .push_str(&types::default(column, default, self.dialect)?);
        }

        if column.unique {
            self.sql.push_str(" UNIQUE");
        }

        Ok(())
    }

    /// Restrictions on `ALTER TABLE .. ADD COLUMN`.
    fn addable(&self, column: &ColumnDef) -> Result<(), SchemaError> {
        let refuse = |reason| SchemaError::AddColumn {
            column: column.name,
            dialect: self.dialect,
            reason,
        };

        if column.generated == Some(Generated::Identity) {
            return Err(refuse("an identity column must be created with its table"));
        }

        if self.dialect == DialectId::Sqlite && column.unique {
            return Err(refuse("a UNIQUE column needs an index instead"));
        }

        if self.dialect == DialectId::Sqlite && !column.nullable && column.default.is_none() {
            return Err(refuse("a NOT NULL column needs a default"));
        }

        Ok(())
    }

    fn foreign_key(
        &mut self,
        table: &TableRef,
        foreign_key: &ForeignKey,
    ) -> Result<(), SchemaError> {
        let references = foreign_key.references;

        if foreign_key.columns.len() != references.columns.len() || references.columns.is_empty() {
            return Err(SchemaError::ForeignKeyArity(table.name));
        }

        self.sql.push_str(", FOREIGN KEY (");
        self.idents(&foreign_key.columns);
        self.sql.push_str(") REFERENCES ");

        if self.dialect == DialectId::Sqlite {
            if references.table.schema.is_some() && references.table.schema != table.schema {
                return Err(SchemaError::CrossSchemaReference(table.name));
            }

            self.ident(references.table.name);
        } else {
            self.table(references.table);
        }

        self.sql.push_str(" (");
        self.idents(references.columns);
        self.sql.push(')');
        self.sql.push_str(match foreign_key.on_delete {
            ReferentialAction::NoAction => "",
            ReferentialAction::Restrict => " ON DELETE RESTRICT",
            ReferentialAction::Cascade => " ON DELETE CASCADE",
            ReferentialAction::SetNull => " ON DELETE SET NULL",
        });

        Ok(())
    }

    fn create_index(&mut self, index: &Index) -> Result<(), SchemaError> {
        if index.columns.is_empty() {
            return Err(SchemaError::EmptyIndex(index.name));
        }

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
            self.sql.push_str(" ON ");
            self.table(index.table);
        }

        self.sql.push_str(" (");
        self.idents(&index.columns);
        self.sql.push(')');

        Ok(())
    }

    /// MySQL cannot key unbounded types.
    fn keyable(&self, table: &Table, columns: &[&'static str]) -> Result<(), SchemaError> {
        if self.dialect != DialectId::MySql {
            return Ok(());
        }

        for name in columns {
            let column = self.find(table, name)?;

            if types::mysql_unbounded(column.ty) {
                return Err(self.unindexable(column));
            }
        }

        Ok(())
    }

    fn unindexable(&self, column: &ColumnDef) -> SchemaError {
        SchemaError::Unindexable {
            column: column.name,
            ty: column.ty,
            dialect: self.dialect,
        }
    }

    fn find<'t>(&self, table: &'t Table, name: &'static str) -> Result<&'t ColumnDef, SchemaError> {
        table
            .columns
            .iter()
            .find(|c| c.name == name)
            .ok_or(SchemaError::UnknownColumn {
                table: table.table.name,
                column: name,
            })
    }

    fn alter(&mut self, table: TableRef) {
        self.sql.push_str("ALTER TABLE ");
        self.table(table);
    }

    fn table(&mut self, table: TableRef) {
        if let Some(schema) = table.schema {
            self.ident(schema);
            self.sql.push('.');
        }

        self.ident(table.name);
    }

    fn idents(&mut self, names: &[&'static str]) {
        for (i, name) in names.iter().enumerate() {
            if i > 0 {
                self.sql.push_str(", ");
            }

            self.ident(name);
        }
    }

    fn ident(&mut self, name: &str) {
        self.syntax.quote_ident(name, &mut self.sql);
    }
}

#[cfg(test)]
mod tests;
