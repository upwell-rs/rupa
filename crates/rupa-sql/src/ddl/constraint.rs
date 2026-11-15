//! `CHECK` and `UNIQUE` constraints: inside `CREATE TABLE` and added to or
//! dropped from an existing table.

use rupa_core::ir::TableRef;
use rupa_core::schema::{Check, Constraint, ConstraintKind, ReferentialAction, Table, Unique};
use rupa_core::{DialectId, Generated};

use super::error::ConstraintChange;
use super::{Ddl, ExprRefusal, SchemaError, types};
use crate::RenderError;

impl Ddl<'_> {
    /// The constraints of `table`, after its keys, checked against its
    /// columns where they are known.
    pub(super) fn table_constraints(&mut self, table: &Table) -> Result<(), RenderError> {
        let restricted = self.restricted_columns(table);

        for constraint in &table.constraints {
            self.belongs_to(table.table, constraint)?;

            if let Constraint::Unique(unique) = constraint {
                self.unique_columns(table, unique)?;
            }

            self.sql.push_str(", ");
            self.constraint(table.table, constraint, &restricted)?;
        }

        Ok(())
    }

    /// `ALTER TABLE .. ADD CONSTRAINT ..`. SQLite has no such statement,
    /// and a rewrite as an index would not be the same constraint.
    pub(super) fn add_constraint(
        &mut self,
        table: TableRef,
        constraint: &Constraint,
    ) -> Result<(), RenderError> {
        self.alterable(constraint.name(), ConstraintChange::Add)?;
        self.belongs_to(table, constraint)?;

        if let Constraint::Unique(unique) = constraint {
            self.unique_types(unique)?;
        }

        self.alter(table);
        self.sql.push_str(" ADD ");
        self.constraint(table, constraint, &[])
    }

    /// Dropping a check is `DROP CHECK` and a unique constraint `DROP INDEX`
    /// on MySQL; PostgreSQL drops both as constraints.
    pub(super) fn drop_constraint(
        &mut self,
        table: TableRef,
        name: &'static str,
        kind: ConstraintKind,
    ) -> Result<(), SchemaError> {
        self.alterable(name, ConstraintChange::Drop)?;
        self.alter(table);

        self.sql.push_str(match (self.dialect, kind) {
            (DialectId::MySql, ConstraintKind::Check) => " DROP CHECK ",
            (DialectId::MySql, ConstraintKind::Unique) => " DROP INDEX ",
            _ => " DROP CONSTRAINT ",
        });
        self.ident(name);

        Ok(())
    }

    fn alterable(
        &self,
        constraint: &'static str,
        change: ConstraintChange,
    ) -> Result<(), SchemaError> {
        if self.dialect == DialectId::Sqlite {
            return Err(SchemaError::ConstraintAlteration {
                constraint,
                dialect: self.dialect,
                change,
            });
        }

        Ok(())
    }

    /// A unique built from another entity's handles would constrain a
    /// same-named column of the wrong table.
    fn belongs_to(&self, table: TableRef, constraint: &Constraint) -> Result<(), SchemaError> {
        match constraint {
            Constraint::Unique(Unique {
                table: Some(recorded),
                ..
            }) if *recorded != table => Err(SchemaError::ConstraintTable {
                constraint: constraint.name(),
                recorded: *recorded,
                expected: table,
            }),
            Constraint::Unique(_) | Constraint::Check(_) => Ok(()),
        }
    }

    fn constraint(
        &mut self,
        table: TableRef,
        constraint: &Constraint,
        restricted: &[ExprRefusal],
    ) -> Result<(), RenderError> {
        self.sql.push_str("CONSTRAINT ");
        self.ident(constraint.name());

        match constraint {
            Constraint::Check(check) => self.check(table, check, restricted)?,
            Constraint::Unique(unique) => self.unique(unique)?,
        }

        Ok(())
    }

    fn check(
        &mut self,
        table: TableRef,
        check: &Check,
        restricted: &[ExprRefusal],
    ) -> Result<(), RenderError> {
        self.sql.push_str(" CHECK (");
        self.restricted_expr(check.name, table, &check.condition, restricted)?;
        self.sql.push(')');

        Ok(())
    }

    /// The columns of `table` MySQL refuses in a check: its `AUTO_INCREMENT`
    /// column and the columns of foreign keys that cascade or set null. Other
    /// dialects have no such restriction.
    fn restricted_columns(&self, table: &Table) -> Vec<ExprRefusal> {
        if self.dialect != DialectId::MySql {
            return Vec::new();
        }

        let auto_increment = table
            .columns
            .iter()
            .filter(|column| column.generated == Some(Generated::Identity))
            .map(|column| ExprRefusal::AutoIncrementColumn {
                column: column.name,
            });
        let referential = table
            .foreign_keys
            .iter()
            .filter(|key| {
                matches!(
                    key.on_delete,
                    ReferentialAction::Cascade | ReferentialAction::SetNull
                )
            })
            .flat_map(|key| &key.columns)
            .map(|column| ExprRefusal::ReferentialActionColumn { column });

        auto_increment.chain(referential).collect()
    }

    fn unique(&mut self, unique: &Unique) -> Result<(), SchemaError> {
        if unique.columns.is_empty() {
            return Err(SchemaError::EmptyConstraint(unique.name));
        }

        let names: Vec<_> = unique.columns.iter().map(|column| column.name).collect();

        self.sql.push_str(" UNIQUE (");
        self.idents(&names);
        self.sql.push(')');

        Ok(())
    }

    /// In `CREATE TABLE` the table's own column definitions decide what a
    /// key may hold, not the types the handles recorded.
    fn unique_columns(&self, table: &Table, unique: &Unique) -> Result<(), SchemaError> {
        let names: Vec<_> = unique.columns.iter().map(|column| column.name).collect();

        for name in &names {
            self.find(table, name)?;
        }

        self.keyable(table, &names)
    }

    /// On an existing table the types are the ones the handles recorded.
    fn unique_types(&self, unique: &Unique) -> Result<(), SchemaError> {
        if self.dialect != DialectId::MySql {
            return Ok(());
        }

        for column in &unique.columns {
            if types::mysql_unbounded(column.ty) {
                return Err(SchemaError::Unindexable {
                    column: column.name,
                    ty: column.ty,
                    dialect: self.dialect,
                });
            }
        }

        Ok(())
    }
}
