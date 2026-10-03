//! The conformance entity, hand-written (the derive arrives in milestone 3).

use chrono::{DateTime, Utc};
use rupa_core::column::{JsonCodec, NullableJsonCodec, ScalarCodec, column_meta};
use rupa_core::ir::TableRef;
use rupa_core::{Column, ColumnMeta, Entity, FromRow, Json, ResultError, Row, Scalar, Value};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    pub score: i64,
    pub flag: bool,
    pub tags: Vec<String>,
    pub nested: Option<Nested>,
    pub note: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Nested {
    pub k: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub id: i64,
    pub name: String,
    pub label: Option<String>,
    pub qty: i32,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub meta: Meta,
    pub extra: Option<Meta>,
}

impl Item {
    pub const ID: Column<Item, i64, Scalar> = Column::scalar("id");
    pub const NAME: Column<Item, String, Scalar> = Column::scalar("name");
    pub const LABEL: Column<Item, Option<String>, Scalar> = Column::scalar("label");
    pub const QTY: Column<Item, i32, Scalar> = Column::scalar("qty");
    pub const ACTIVE: Column<Item, bool, Scalar> = Column::scalar("active");
    pub const CREATED_AT: Column<Item, DateTime<Utc>, Scalar> = Column::scalar("created");
    pub const META: Column<Item, Meta, Json> = Column::json("meta");
    pub const EXTRA: Column<Item, Option<Meta>, Json> = Column::nullable_json("extra");
}

static ITEM_COLUMNS: [ColumnMeta; 8] = [
    column_meta::<i64, ScalarCodec<i64>>("id", "id"),
    column_meta::<String, ScalarCodec<String>>("name", "name"),
    column_meta::<Option<String>, ScalarCodec<Option<String>>>("label", "label"),
    column_meta::<i32, ScalarCodec<i32>>("qty", "qty"),
    column_meta::<bool, ScalarCodec<bool>>("active", "active"),
    column_meta::<DateTime<Utc>, ScalarCodec<DateTime<Utc>>>("created_at", "created"),
    column_meta::<Meta, JsonCodec<Meta>>("meta", "meta"),
    column_meta::<Option<Meta>, NullableJsonCodec<Meta>>("extra", "extra"),
];

impl FromRow for Item {
    fn from_row(row: &dyn Row) -> Result<Self, ResultError> {
        Ok(Item {
            id: Self::ID.read(row, 0)?,
            name: Self::NAME.read(row, 1)?,
            label: Self::LABEL.read(row, 2)?,
            qty: Self::QTY.read(row, 3)?,
            active: Self::ACTIVE.read(row, 4)?,
            created_at: Self::CREATED_AT.read(row, 5)?,
            meta: Self::META.read(row, 6)?,
            extra: Self::EXTRA.read(row, 7)?,
        })
    }
}

impl Entity for Item {
    type Id = i64;
    const TABLE: TableRef = TableRef::with_schema("conformance", "items");
    const ID_COLUMNS: &'static [&'static str] = &["id"];

    fn columns() -> &'static [ColumnMeta] {
        &ITEM_COLUMNS
    }

    fn id(&self) -> &i64 {
        &self.id
    }

    fn to_values(&self) -> Vec<Value> {
        vec![
            Self::ID.encode(&self.id),
            Self::NAME.encode(&self.name),
            Self::LABEL.encode(&self.label),
            Self::QTY.encode(&self.qty),
            Self::ACTIVE.encode(&self.active),
            Self::CREATED_AT.encode(&self.created_at),
            Self::META.encode(&self.meta),
            Self::EXTRA.encode(&self.extra),
        ]
    }
}

/// Postgres schema for the conformance entities. Drops and recreates.
pub const POSTGRES_DDL: &str = r#"
DROP SCHEMA IF EXISTS conformance CASCADE;
CREATE SCHEMA conformance;
CREATE TABLE conformance.items (
    id      bigint PRIMARY KEY,
    name    text NOT NULL,
    label   text,
    qty     integer NOT NULL,
    active  boolean NOT NULL,
    created timestamptz NOT NULL,
    meta    jsonb NOT NULL,
    extra   jsonb
);
"#;

fn at(secs: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(secs, 0).expect("valid timestamp")
}

/// The seed rows every case starts from.
pub fn items() -> Vec<Item> {
    let meta = |score, flag, tags: &[&str], nested: Option<&str>| Meta {
        score,
        flag,
        tags: tags.iter().map(|t| t.to_string()).collect(),
        nested: nested.map(|k| Nested { k: k.into() }),
        note: None,
    };
    vec![
        Item {
            id: 1,
            name: "apple".into(),
            label: Some("red".into()),
            qty: 3,
            active: true,
            created_at: at(1_700_000_000),
            meta: meta(5, true, &["a", "b"], None),
            extra: None,
        },
        Item {
            id: 2,
            name: "banana".into(),
            label: None,
            qty: 12,
            active: false,
            created_at: at(1_700_000_100),
            meta: meta(7, false, &[], Some("v")),
            extra: Some(meta(1, true, &["x"], None)),
        },
        Item {
            id: 3,
            name: "cherry".into(),
            label: Some("dark_red".into()),
            qty: 7,
            active: true,
            created_at: at(1_700_000_200),
            meta: meta(9, true, &["c"], Some("w")),
            extra: None,
        },
        Item {
            id: 4,
            name: "date".into(),
            label: None,
            qty: 0,
            active: true,
            created_at: at(1_700_000_300),
            meta: meta(-1, false, &[], None),
            extra: None,
        },
    ]
}
