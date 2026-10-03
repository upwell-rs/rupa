//! A hand-written entity, as a user would write one without `#[derive(Entity)]`.
//! Shared by rupa-core's tests and rupa-sql's snapshot tests (via `#[path]`).
#![allow(dead_code)]

use chrono::{DateTime, Utc};
use rupa_core::column::{JsonCodec, NullableJsonCodec, ScalarCodec, column_meta};
use rupa_core::ir::TableRef;
use rupa_core::{Column, ColumnMeta, Entity, FromRow, Json, ResultError, Row, Scalar, Value};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Prefs {
    pub theme: String,
    pub beta: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct User {
    pub id: i64,
    pub email: String,
    pub nickname: Option<String>,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub prefs: Prefs,
    pub old_prefs: Option<Prefs>,
}

impl User {
    pub const ID: Column<User, i64, Scalar> = Column::scalar("id");
    pub const EMAIL: Column<User, String, Scalar> = Column::scalar("email");
    pub const NICKNAME: Column<User, Option<String>, Scalar> = Column::scalar("nickname");
    pub const ACTIVE: Column<User, bool, Scalar> = Column::scalar("active");
    pub const CREATED_AT: Column<User, DateTime<Utc>, Scalar> = Column::scalar("created");
    pub const PREFS: Column<User, Prefs, Json> = Column::json("prefs");
    pub const OLD_PREFS: Column<User, Option<Prefs>, Json> = Column::nullable_json("old_prefs");

    pub fn sample() -> Self {
        User {
            id: 7,
            email: "a@b.c".into(),
            nickname: None,
            active: true,
            created_at: DateTime::from_timestamp(1_700_000_000, 0).unwrap(),
            prefs: Prefs {
                theme: "dark".into(),
                beta: true,
            },
            old_prefs: None,
        }
    }
}

static USER_COLUMNS: [ColumnMeta; 7] = [
    column_meta::<i64, ScalarCodec<i64>>("id", "id"),
    column_meta::<String, ScalarCodec<String>>("email", "email"),
    column_meta::<Option<String>, ScalarCodec<Option<String>>>("nickname", "nickname"),
    column_meta::<bool, ScalarCodec<bool>>("active", "active"),
    column_meta::<DateTime<Utc>, ScalarCodec<DateTime<Utc>>>("created_at", "created"),
    column_meta::<Prefs, JsonCodec<Prefs>>("prefs", "prefs"),
    column_meta::<Option<Prefs>, NullableJsonCodec<Prefs>>("old_prefs", "old_prefs"),
];

impl FromRow for User {
    fn from_row(row: &dyn Row) -> Result<Self, ResultError> {
        Ok(User {
            id: Self::ID.read(row, 0)?,
            email: Self::EMAIL.read(row, 1)?,
            nickname: Self::NICKNAME.read(row, 2)?,
            active: Self::ACTIVE.read(row, 3)?,
            created_at: Self::CREATED_AT.read(row, 4)?,
            prefs: Self::PREFS.read(row, 5)?,
            old_prefs: Self::OLD_PREFS.read(row, 6)?,
        })
    }
}

impl Entity for User {
    type Id = i64;
    const TABLE: TableRef = TableRef::with_schema("app", "users");
    const ID_COLUMNS: &'static [&'static str] = &["id"];

    fn columns() -> &'static [ColumnMeta] {
        &USER_COLUMNS
    }

    fn id(&self) -> &i64 {
        &self.id
    }

    fn to_values(&self) -> Vec<Value> {
        vec![
            Self::ID.encode(&self.id),
            Self::EMAIL.encode(&self.email),
            Self::NICKNAME.encode(&self.nickname),
            Self::ACTIVE.encode(&self.active),
            Self::CREATED_AT.encode(&self.created_at),
            Self::PREFS.encode(&self.prefs),
            Self::OLD_PREFS.encode(&self.old_prefs),
        ]
    }
}

/// What `#[derive(Entity)]` will emit for `col!` (kinds left to inference).
pub struct UserFields {
    pub email: rupa_core::__private::Probe<User, String>,
    pub nickname: rupa_core::__private::Probe<User, Option<String>>,
    pub prefs: rupa_core::__private::Probe<User, Prefs>,
    pub old_prefs: rupa_core::__private::Probe<User, Option<Prefs>>,
}

impl User {
    pub const fn __rupa_fields() -> UserFields {
        use rupa_core::__private::Probe;
        UserFields {
            email: Probe::new("email"),
            nickname: Probe::new("nickname"),
            prefs: Probe::new("prefs"),
            old_prefs: Probe::new("old_prefs"),
        }
    }
}
