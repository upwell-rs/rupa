//! Bounded text fields keep their length in the entity's column metadata.

use rupa::VarChar;
use rupa::core::schema::{ColumnDef, ColumnType, Index, IndexColumn};
use rupa::prelude::*;

#[derive(Debug, Clone, PartialEq, Entity)]
#[entity(table = "accounts")]
struct Account {
    #[id(generated)]
    id: i64,
    email: VarChar<255>,
    nickname: Option<VarChar<16>>,
    bio: String,
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
