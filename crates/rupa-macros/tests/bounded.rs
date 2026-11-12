//! Bounded text fields keep their length in the entity's column metadata.

use rupa::VarChar;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Profile {
    theme: String,
}
use rupa::core::schema::{ColumnDef, ColumnType, Gin, Index, IndexColumn, IndexKind};
use rupa::prelude::*;

#[derive(Debug, Clone, PartialEq, Entity)]
#[entity(table = "accounts")]
struct Account {
    #[id(generated)]
    id: i64,
    email: VarChar<255>,
    nickname: Option<VarChar<16>>,
    bio: String,
    profile: Profile,
    labels: Vec<String>,
    prefs: Option<Profile>,
    raw: serde_json::Value,
}

#[test]
fn varchar_fields_declare_their_length_in_the_schema() {
    let columns: Vec<_> = Account::COLUMNS.iter().map(ColumnDef::from_meta).collect();

    assert_eq!(
        columns[1],
        ColumnDef::new("email", ColumnType::VarChar(255))
    );
    assert_eq!(
        columns[2],
        ColumnDef::new("nickname", ColumnType::VarChar(16)).nullable()
    );
    assert_eq!(columns[3], ColumnDef::new("bio", ColumnType::Text));
}

#[test]
fn handles_carry_the_length_into_indexes() {
    assert_eq!(
        Index::on("accounts_email", Account::EMAIL).columns,
        [IndexColumn::new("email", ColumnType::VarChar(255))]
    );
    assert_eq!(
        Index::on("accounts_bio", Account::BIO).columns,
        [IndexColumn::new("bio", ColumnType::Text)]
    );
}

#[test]
fn varchar_values_compare_in_queries() {
    let email = VarChar::<255>::new("ada@example.com").expect("short enough");
    let _ = select::<Account>().filter(Account::EMAIL.eq(email));
}

#[test]
fn gin_accepts_every_json_column_without_user_impls() {
    let index = Gin::on("accounts_profile", Account::PROFILE)
        .and(Account::LABELS)
        .and(Account::PREFS)
        .and(Account::RAW)
        .build();

    assert_eq!(index.kind, Gin::METHOD);
    assert_eq!(
        index.columns.iter().map(|c| c.name).collect::<Vec<_>>(),
        ["profile", "labels", "prefs", "raw"]
    );
}
