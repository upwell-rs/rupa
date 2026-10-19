//! Canonical checksum records for schema operations.
//!
//! Part of [`CHECKSUM_FORMAT`](super::CHECKSUM_FORMAT): every field is a
//! length-prefixed record in a fixed order, behind a tag naming its variant.
//! Changing any encoding changes applied checksums, so it requires a new
//! format identity. The exhaustive matches make every new IR variant decide
//! its encoding.

use rupa_core::ir::TableRef;
use rupa_core::schema::{
    ColumnDef, ColumnDefault, ColumnType, ForeignKey, Index, ReferentialAction, SchemaOp, Table,
};
use rupa_core::{Generated, Value};
use sha2::Sha256;

use super::hash_field;

/// Writes canonical records into a running digest.
pub(super) struct Encoder<'h>(pub(super) &'h mut Sha256);

impl Encoder<'_> {
    pub(super) fn schema(&mut self, op: &SchemaOp) {
        match op {
            SchemaOp::CreateTable(table) => {
                self.str("create-table");
                self.table(table);
            }
            SchemaOp::DropTable(table) => {
                self.str("drop-table");
                self.table_ref(*table);
            }
            SchemaOp::RenameTable { table, to } => {
                self.str("rename-table");
                self.table_ref(*table);
                self.str(to);
            }
            SchemaOp::AddColumn { table, column } => {
                self.str("add-column");
                self.table_ref(*table);
                self.column(column);
            }
            SchemaOp::DropColumn { table, column } => {
                self.str("drop-column");
                self.table_ref(*table);
                self.str(column);
            }
            SchemaOp::RenameColumn { table, from, to } => {
                self.str("rename-column");
                self.table_ref(*table);
                self.str(from);
                self.str(to);
            }
            SchemaOp::CreateIndex(index) => {
                self.str("create-index");
                self.index(index);
            }
            SchemaOp::DropIndex { table, name } => {
                self.str("drop-index");
                self.table_ref(*table);
                self.str(name);
            }
        }
    }

    fn table(&mut self, table: &Table) {
        self.table_ref(table.table);
        self.len(table.columns.len());

        for column in &table.columns {
            self.column(column);
        }

        self.names(&table.primary_key);
        self.len(table.foreign_keys.len());

        for foreign_key in &table.foreign_keys {
            self.foreign_key(foreign_key);
        }
    }

    fn column(&mut self, column: &ColumnDef) {
        self.str(column.name);
        self.ty(column.ty);
        self.flag(column.nullable);
        self.str(match column.generated {
            None => "stored",
            Some(Generated::Identity) => "identity",
            Some(Generated::Database) => "database",
        });

        match &column.default {
            None => self.str("no-default"),
            Some(ColumnDefault::Sql(sql)) => {
                self.str("sql-default");
                self.str(sql);
            }
            Some(ColumnDefault::Value(value)) => {
                self.str("value-default");
                self.value(value);
            }
        }

        self.flag(column.unique);
    }

    fn ty(&mut self, ty: ColumnType) {
        match ty {
            ColumnType::Bool => self.str("bool"),
            ColumnType::SmallInt => self.str("smallint"),
            ColumnType::Int => self.str("int"),
            ColumnType::BigInt => self.str("bigint"),
            ColumnType::Real => self.str("real"),
            ColumnType::Double => self.str("double"),
            ColumnType::Decimal { precision, scale } => {
                self.str("decimal");
                self.len(precision.into());
                self.len(scale.into());
            }
            ColumnType::Text => self.str("text"),
            ColumnType::VarChar(n) => {
                self.str("varchar");
                self.len(n as usize);
            }
            ColumnType::Bytes => self.str("bytes"),
            ColumnType::Uuid => self.str("uuid"),
            ColumnType::Date => self.str("date"),
            ColumnType::Time => self.str("time"),
            ColumnType::Timestamp => self.str("timestamp"),
            ColumnType::TimestampTz => self.str("timestamptz"),
            ColumnType::Json => self.str("json"),
            ColumnType::Custom(custom) => {
                self.str("custom");

                for name in [custom.postgres, custom.mysql, custom.sqlite] {
                    self.opt(name);
                }
            }
        }
    }

    /// Default literals. Values no dialect can write as a default are still
    /// recorded distinctly; validation rejects them before execution.
    fn value(&mut self, value: &Value) {
        match value {
            Value::Bool(b) => {
                self.str("bool");
                self.flag(*b);
            }
            Value::I16(n) => self.number("i16", &n.to_be_bytes()),
            Value::I32(n) => self.number("i32", &n.to_be_bytes()),
            Value::I64(n) => self.number("i64", &n.to_be_bytes()),
            Value::F32(n) => self.number("f32", &n.to_bits().to_be_bytes()),
            Value::F64(n) => self.number("f64", &n.to_bits().to_be_bytes()),
            Value::Text(text) => {
                self.str("text");
                self.str(text);
            }
            Value::Json(json) => {
                self.str("json");
                self.str(&json.to_string());
            }
            other => {
                self.str("other");
                self.str(&format!("{other:?}"));
            }
        }
    }

    fn foreign_key(&mut self, foreign_key: &ForeignKey) {
        self.names(&foreign_key.columns);
        self.table_ref(foreign_key.references.table);
        self.names(foreign_key.references.columns);
        self.str(match foreign_key.on_delete {
            ReferentialAction::NoAction => "no-action",
            ReferentialAction::Restrict => "restrict",
            ReferentialAction::Cascade => "cascade",
            ReferentialAction::SetNull => "set-null",
        });
    }

    fn index(&mut self, index: &Index) {
        self.str(index.name);
        self.table_ref(index.table);
        self.names(&index.columns);
        self.flag(index.unique);
    }

    fn table_ref(&mut self, table: TableRef) {
        self.opt(table.schema);
        self.str(table.name);
    }

    fn names(&mut self, names: &[&str]) {
        self.len(names.len());

        for name in names {
            self.str(name);
        }
    }

    fn number(&mut self, tag: &str, bytes: &[u8]) {
        self.str(tag);
        hash_field(self.0, bytes);
    }

    fn opt(&mut self, value: Option<&str>) {
        match value {
            Some(value) => {
                self.str("some");
                self.str(value);
            }
            None => self.str("none"),
        }
    }

    fn flag(&mut self, flag: bool) {
        self.str(if flag { "true" } else { "false" });
    }

    fn len(&mut self, len: usize) {
        hash_field(self.0, &(len as u64).to_be_bytes());
    }

    fn str(&mut self, value: &str) {
        hash_field(self.0, value.as_bytes());
    }
}
