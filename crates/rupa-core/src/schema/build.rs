//! Builders for schema definitions, from literals or from entities.

use super::{
    ColumnDef, ColumnDefault, ColumnType, CustomType, ForeignKey, Index, ReferentialAction, Table,
};
use crate::column::{Column, ColumnMeta, Generated, Reference};
use crate::entity::Entity;
use crate::ir::TableRef;

impl ColumnDef {
    /// A `NOT NULL` column of type `ty`.
    pub const fn new(name: &'static str, ty: ColumnType) -> Self {
        Self {
            name,
            ty,
            nullable: false,
            generated: None,
            default: None,
            unique: false,
        }
    }

    /// The column an entity field handle describes: its name, its type
    /// (from the field's SQL type) and its nullability.
    pub const fn of<E, T, K>(column: Column<E, T, K>) -> Self {
        let mut def = Self::new(column.name(), ColumnType::from_sql(column.sql_type()));

        def.nullable = column.nullable();

        def
    }

    /// The column an entity's metadata describes, including generation.
    pub const fn from_meta(meta: &ColumnMeta) -> Self {
        let mut def = Self::new(meta.name, ColumnType::from_sql(meta.sql_type));

        def.nullable = meta.nullable;
        def.generated = meta.generated;

        def
    }

    /// Admits `NULL`.
    pub const fn nullable(mut self) -> Self {
        self.nullable = true;

        self
    }

    /// Rejects `NULL`.
    pub const fn not_null(mut self) -> Self {
        self.nullable = false;

        self
    }

    /// Replaces the type, e.g. `Text` by `VarChar(255)` for an indexed column.
    pub const fn ty(mut self, ty: ColumnType) -> Self {
        self.ty = ty;

        self
    }

    /// An integer identity (auto-increment) column.
    pub const fn identity(mut self) -> Self {
        self.generated = Some(Generated::Identity);

        self
    }

    /// Sets the default value.
    pub fn default(mut self, default: ColumnDefault) -> Self {
        self.default = Some(default);

        self
    }

    /// Adds a single-column `UNIQUE` constraint.
    pub const fn unique(mut self) -> Self {
        self.unique = true;

        self
    }
}

impl ColumnType {
    /// [`From<SqlType>`], usable in constants.
    pub const fn from_sql(ty: crate::value::SqlType) -> Self {
        use crate::value::SqlType;

        match ty {
            SqlType::Bool => Self::Bool,
            SqlType::I16 => Self::SmallInt,
            SqlType::I32 => Self::Int,
            SqlType::I64 => Self::BigInt,
            SqlType::F32 => Self::Real,
            SqlType::F64 => Self::Double,
            SqlType::Text => Self::Text,
            SqlType::Bytes => Self::Bytes,
            SqlType::Uuid => Self::Uuid,
            SqlType::Date => Self::Date,
            SqlType::Time => Self::Time,
            SqlType::Timestamp => Self::Timestamp,
            SqlType::TimestampTz => Self::TimestampTz,
            SqlType::Json => Self::Json,
        }
    }
}

impl CustomType {
    /// No dialect has a name yet.
    pub const fn new() -> Self {
        Self {
            postgres: None,
            mysql: None,
            sqlite: None,
        }
    }

    /// The PostgreSQL type name.
    pub const fn postgres(mut self, name: &'static str) -> Self {
        self.postgres = Some(name);

        self
    }

    /// The MySQL type name.
    pub const fn mysql(mut self, name: &'static str) -> Self {
        self.mysql = Some(name);

        self
    }

    /// The SQLite type name.
    pub const fn sqlite(mut self, name: &'static str) -> Self {
        self.sqlite = Some(name);

        self
    }
}

impl Table {
    /// An empty table in the default schema.
    pub fn named(name: &'static str) -> Self {
        Self::at(TableRef::new(name))
    }

    /// An empty table at `table`.
    pub fn at(table: TableRef) -> Self {
        Self {
            table,
            columns: Vec::new(),
            primary_key: Vec::new(),
            foreign_keys: Vec::new(),
        }
    }

    /// The table entity `E` describes: its columns, primary key, identity
    /// and foreign keys, as they are when the plan is built.
    ///
    /// The definition follows `E`: if `E` changes later, so does every plan
    /// built from it, and an applied migration's checksum stops matching.
    /// Declare frozen tables with [`Table::named`] or a migration-local entity.
    pub fn of<E: Entity>() -> Self {
        let mut table = Self::at(E::TABLE);

        for meta in E::COLUMNS {
            table.columns.push(ColumnDef::from_meta(meta));

            if meta.primary_key {
                table.primary_key.push(meta.name);
            }

            if let Some(references) = meta.references {
                table
                    .foreign_keys
                    .push(ForeignKey::new([meta.name], references));
            }
        }

        table
    }

    /// Adds `column`, or replaces the column of the same name.
    pub fn column(mut self, column: ColumnDef) -> Self {
        match self.columns.iter_mut().find(|c| c.name == column.name) {
            Some(existing) => *existing = column,
            None => self.columns.push(column),
        }

        self
    }

    /// Removes the column `name`, with its primary key and foreign key parts.
    pub fn without(mut self, name: &str) -> Self {
        self.columns.retain(|c| c.name != name);
        self.primary_key.retain(|c| *c != name);
        self.foreign_keys.retain(|f| !f.columns.contains(&name));

        self
    }

    /// Sets the primary key columns.
    pub fn primary_key(mut self, columns: impl IntoIterator<Item = &'static str>) -> Self {
        self.primary_key = columns.into_iter().collect();

        self
    }

    /// Adds `foreign_key`, replacing one over the same columns.
    pub fn foreign_key(mut self, foreign_key: ForeignKey) -> Self {
        self.foreign_keys
            .retain(|f| f.columns != foreign_key.columns);
        self.foreign_keys.push(foreign_key);

        self
    }
}

impl ForeignKey {
    /// `columns` referencing `references`, with no delete action.
    pub fn new(columns: impl IntoIterator<Item = &'static str>, references: Reference) -> Self {
        Self {
            columns: columns.into_iter().collect(),
            references,
            on_delete: ReferentialAction::NoAction,
        }
    }

    /// `columns` referencing the id of entity `E`.
    pub fn to<E: Entity>(columns: impl IntoIterator<Item = &'static str>) -> Self {
        Self::new(
            columns,
            Reference {
                table: E::TABLE,
                columns: E::ID_COLUMNS,
            },
        )
    }

    /// Sets what deleting a referenced row does.
    pub fn on_delete(mut self, action: ReferentialAction) -> Self {
        self.on_delete = action;

        self
    }
}

impl Index {
    /// An index named `name` over `columns` of `table`.
    pub fn new(
        name: &'static str,
        table: TableRef,
        columns: impl IntoIterator<Item = &'static str>,
    ) -> Self {
        Self {
            name,
            table,
            columns: columns.into_iter().collect(),
            unique: false,
        }
    }

    /// An index named `name` over one entity column.
    pub fn on<E: Entity, T, K>(name: &'static str, column: Column<E, T, K>) -> Self {
        Self::new(name, E::TABLE, [column.name()])
    }

    /// An index named `name` over `columns` of entity `E`'s table.
    pub fn of<E: Entity>(
        name: &'static str,
        columns: impl IntoIterator<Item = &'static str>,
    ) -> Self {
        Self::new(name, E::TABLE, columns)
    }

    /// Makes indexed values unique.
    pub fn unique(mut self) -> Self {
        self.unique = true;

        self
    }
}
