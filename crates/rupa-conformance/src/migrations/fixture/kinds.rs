//! Column kinds and the tables built from them.

use rupa::core::ScalarColumn;
use rupa::migrate::Plan;
use rupa::migrate::schema::{ColumnKind, ColumnSpec, ColumnType, Table, TypeSupport};
use rupa::{DecodeError, SqlType, Value};

/// Long text stored under a different type name on every dialect.
#[derive(Debug, Clone, PartialEq)]
pub struct BigText(pub String);

impl ColumnKind for BigText {
    const SPEC: ColumnSpec = ColumnSpec::new("big-text", SqlType::Text)
        .postgres(TypeSupport::named("TEXT"))
        .mysql(TypeSupport::named("MEDIUMTEXT"))
        .sqlite(TypeSupport::named("CLOB"));
}

/// A text-search document, which only PostgreSQL stores. Its values need a
/// cast the query IR cannot write yet (#91): the fixture only declares it.
#[derive(Debug, Clone, PartialEq)]
pub struct Tsvector(pub String);

impl ColumnKind for Tsvector {
    const SPEC: ColumnSpec = ColumnSpec::new("tsvector", SqlType::Text)
        .postgres(TypeSupport::named("TSVECTOR"))
        .mysql(TypeSupport::Unsupported("MySQL has no tsvector type"))
        .sqlite(TypeSupport::Unsupported("SQLite has no tsvector type"));
}

macro_rules! text_kind_column {
    ($($kind:ident),*) => {$(
        impl ScalarColumn for $kind {
            type Base = Self;
            const SQL_TYPE: SqlType = Self::SPEC.sql_type;
            const COLUMN_TYPE: ColumnType = Self::SPEC.column_type();

            fn to_value(&self) -> Value {
                Value::Text(self.0.clone())
            }

            fn from_value(v: Value) -> Result<Self, DecodeError> {
                match v {
                    Value::Text(text) => Ok(Self(text)),
                    other => Err(DecodeError::type_mismatch(SqlType::Text, &other)),
                }
            }
        }
    )*};
}

text_kind_column!(BigText, Tsvector);

/// A note whose body is a [`BigText`] column.
#[derive(Debug, PartialEq, rupa::Entity)]
#[entity(table = "kind_notes")]
pub struct KindNote {
    #[id(generated)]
    pub id: i64,
    pub body: BigText,
}

/// Typed insert input for [`KindNote`]: no id.
#[derive(Debug, Clone, rupa::Insertable)]
#[insertable(entity = KindNote)]
pub struct NewKindNote {
    pub body: BigText,
}

/// A document with a [`Tsvector`] column.
#[derive(Debug, rupa::Entity)]
#[entity(table = "kind_documents")]
pub struct KindDocument {
    #[id(generated)]
    pub id: i64,
    pub terms: Tsvector,
}

/// Creates a table with a kind every dialect stores.
pub fn create_big_text_table() -> Plan {
    Plan::new().create_table(Table::of::<KindNote>())
}

/// Creates a table with a kind only PostgreSQL stores.
pub fn create_tsvector_table() -> Plan {
    Plan::new().create_table(Table::of::<KindDocument>())
}
