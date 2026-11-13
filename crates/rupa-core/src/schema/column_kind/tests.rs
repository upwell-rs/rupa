use super::*;
use crate::column::{Column, Scalar};
use crate::entity::{Entity, FromRow};
use crate::error::{DecodeError, ResultError};
use crate::ir::TableRef;
use crate::row::Row;
use crate::schema::{ColumnDef, Gin, IndexKind, IndexableBy, column};
use crate::value::{ScalarColumn, Value};

struct Vector<const N: usize>(Vec<f32>);

impl<const N: usize> ColumnKind for Vector<N> {
    const SPEC: ColumnSpec = ColumnSpec::new("vector", SqlType::Text)
        .postgres(TypeSupport::with_args("vector", &[N as u32]))
        .mysql(TypeSupport::Unsupported("MySQL has no vector type"))
        .sqlite(TypeSupport::named("TEXT"))
        .bounded();
}

impl<const N: usize> ScalarColumn for Vector<N> {
    type Base = Self;
    const SQL_TYPE: SqlType = Self::SPEC.sql_type;
    const COLUMN_TYPE: ColumnType = Self::SPEC.column_type();

    fn to_value(&self) -> Value {
        Value::Text(format!("{:?}", self.0))
    }

    fn from_value(v: Value) -> Result<Self, DecodeError> {
        Err(DecodeError::type_mismatch(SqlType::Text, &v))
    }
}

impl<const N: usize> IndexableBy<Gin> for Vector<N> {}

struct Item;

impl Item {
    const EMBEDDING: Column<Item, Vector<3>, Scalar> =
        Column::scalar_in(<Item as Entity>::TABLE, "embedding");
    const MAYBE: Column<Item, Option<Vector<8>>, Scalar> =
        Column::scalar_in(<Item as Entity>::TABLE, "maybe");
}

impl FromRow for Item {
    fn from_row(_row: &dyn Row) -> Result<Self, ResultError> {
        Ok(Self)
    }
}

impl Entity for Item {
    type Id = i64;
    const TABLE: TableRef = TableRef::new("items");
    const ID_COLUMNS: &'static [&'static str] = &["id"];
    const COLUMNS: &'static [crate::column::ColumnMeta] = &[];

    fn id(&self) -> &i64 {
        &0
    }

    fn to_values(&self) -> Vec<Value> {
        Vec::new()
    }
}

#[test]
fn const_generics_become_structured_arguments() {
    assert_eq!(
        Vector::<3>::SPEC.postgres,
        TypeSupport::with_args("vector", &[3])
    );
    assert_ne!(Vector::<3>::SPEC, Vector::<4>::SPEC);
    assert_eq!(Vector::<3>::SPEC, Vector::<3>::SPEC);
}

#[test]
fn support_follows_the_dialect() {
    let spec = Vector::<3>::SPEC;

    assert_eq!(spec.support(DialectId::Postgres), spec.postgres);
    assert_eq!(spec.support(DialectId::MySql), spec.mysql);
    assert_eq!(spec.support(DialectId::Sqlite), spec.sqlite);
}

#[test]
fn a_dialect_is_unsupported_until_declared() {
    let spec = ColumnSpec::new("bare", SqlType::Text);

    assert!(matches!(spec.postgres, TypeSupport::Unsupported(_)));
    assert!(matches!(spec.mysql, TypeSupport::Unsupported(_)));
    assert!(matches!(spec.sqlite, TypeSupport::Unsupported(_)));
    assert!(!spec.bounded);
}

#[test]
fn kinds_are_entity_field_and_literal_column_types() {
    let from_handle = ColumnDef::of(Item::EMBEDDING);
    let nullable = ColumnDef::of(Item::MAYBE);

    assert_eq!(from_handle.ty, ColumnType::Kind(&Vector::<3>::SPEC));
    assert!(!from_handle.nullable);
    assert_eq!(nullable.ty, ColumnType::Kind(&Vector::<8>::SPEC));
    assert!(nullable.nullable);
    assert_eq!(
        column::<Vector<3>>("embedding"),
        ColumnDef::new("embedding", ColumnType::Kind(&Vector::<3>::SPEC))
    );
}

#[test]
fn indexes_over_kind_columns_use_the_kinds_opt_in() {
    let index = Gin::on("items_embedding", Item::EMBEDDING).build();

    assert_eq!(
        index.columns().next().expect("one column").ty,
        ColumnType::Kind(&Vector::<3>::SPEC)
    );
}
