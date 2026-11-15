use rupa_core::ir::TableRef;
use rupa_core::schema::{ColumnDef, ColumnType, ForeignKey, Index, IndexColumn, Table, Unique};
use rupa_core::{Reference, Relation, VarChar};

use super::plans::{assert_distinct_digests, plan_digest};
use super::*;

rupa_macros::table!(
    #[rupa(crate = rupa_core)]
    Authors = "main"."authors" {
        #[id]
        id: i64,
    }
);

rupa_macros::table!(
    #[rupa(crate = rupa_core)]
    Accounts = "main"."accounts" {
        #[id(generated)]
        id: i64,
        #[column(references = Authors)]
        author_id: Option<i64>,
        nick: Option<VarChar<64>>,
        email: String,
        prefs: serde_json::Value,
    }
);

fn literal_accounts() -> Table {
    Table::at(TableRef::with_schema("main", "accounts"))
        .column(ColumnDef::new("id", ColumnType::BigInt).identity())
        .column(ColumnDef::new("author_id", ColumnType::BigInt).nullable())
        .column(ColumnDef::new("nick", ColumnType::VarChar(64)).nullable())
        .column(ColumnDef::new("email", ColumnType::Text))
        .column(ColumnDef::new("prefs", ColumnType::Json))
        .primary_key(["id"])
        .foreign_key(ForeignKey::new(
            ["author_id"],
            Reference {
                table: TableRef::with_schema("main", "authors"),
                columns: &["id"],
            },
        ))
}

#[test]
fn a_typed_table_is_the_literal_table_it_describes() {
    assert_eq!(Table::of::<Accounts>(), literal_accounts());
}

#[test]
fn a_typed_table_has_the_checksum_of_its_literal_definition() {
    let typed = plan_digest(|| Plan::new().create_table(Table::of::<Accounts>()));
    let literal = plan_digest(|| Plan::new().create_table(literal_accounts()));

    assert_eq!(typed, literal);
}

#[test]
fn typed_columns_and_constraints_keep_the_checksum_of_their_literals() {
    let typed = plan_digest(|| {
        Plan::new()
            .add_column(Accounts::NICK)
            .create_index(Index::on("accounts_email", Accounts::EMAIL).unique())
            .add_constraint(
                Accounts::TABLE,
                Unique::on("accounts_email_nick", Accounts::EMAIL).and(Accounts::NICK),
            )
    });
    let literal = plan_digest(|| {
        let table = TableRef::with_schema("main", "accounts");

        Plan::new()
            .add_column_to(
                table,
                ColumnDef::new("nick", ColumnType::VarChar(64)).nullable(),
            )
            .create_index(
                Index::new(
                    "accounts_email",
                    table,
                    [IndexColumn::new("email", ColumnType::Text)],
                )
                .unique(),
            )
            .add_constraint(
                table,
                Unique::new(
                    "accounts_email_nick",
                    [
                        IndexColumn::new("email", ColumnType::Text),
                        IndexColumn::new("nick", ColumnType::VarChar(64)),
                    ],
                ),
            )
    });

    assert_eq!(typed, literal);
}

#[test]
fn a_changed_typed_table_changes_the_checksum() {
    let variants: [fn() -> Plan; 2] = [
        || Plan::new().create_table(Table::of::<Accounts>()),
        || Plan::new().create_table(Table::of::<Accounts>().without("nick")),
    ];

    assert_distinct_digests(
        &variants,
        "dropping a typed column must change the checksum",
    );
}
