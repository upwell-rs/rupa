//! `rupa::table!`: typed table snapshots with no row type.

use rupa::core::ir::TableRef;
use rupa::core::schema::Table;
use rupa::prelude::*;
use rupa::{Postgres, VarChar};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Prefs {
    theme: String,
}

#[derive(Debug, Entity)]
#[entity(table = "owners", schema = "main")]
struct Owner {
    #[id(generated)]
    id: i64,
}

#[derive(Debug, Entity)]
#[entity(table = "accounts", schema = "main")]
struct AccountEntity {
    #[id(generated)]
    id: i64,
    #[column(references = Owner)]
    owner_id: i64,
    nick: Option<VarChar<64>>,
    email: String,
    prefs: Prefs,
    old_prefs: Option<Prefs>,
}

rupa::table!(
    Accounts = "main"."accounts" {
        #[id(generated)]
        id: i64,
        #[column(references = Owner)]
        owner_id: i64,
        nick: Option<VarChar<64>>,
        email: String,
        prefs: Prefs,
        old_prefs: Option<Prefs>,
    }
);

rupa::table!(
    /// A table with a composite key and no schema.
    pub(crate) Memberships = "memberships" {
        #[id]
        team_id: i64,
        #[id]
        user_id: i64,
        role: String,
    }
);

#[derive(Debug, Entity)]
#[entity(table = "audit", schema = "main")]
struct AuditEntity {
    #[id(generated)]
    id: i64,
    #[column(name = "created")]
    created_at: String,
    #[column(generated)]
    version: i32,
    #[column(json)]
    raw: String,
    #[column(scalar)]
    label: String,
    #[column(name = "owner", references = Owner)]
    owner_id: i64,
}

rupa::table!(
    Audit = "main"."audit" {
        #[id(generated)]
        id: i64,
        #[column(name = "created")]
        created_at: String,
        #[column(generated)]
        version: i32,
        #[column(json)]
        raw: String,
        #[column(scalar)]
        label: String,
        #[column(name = "owner", references = Owner)]
        owner_id: i64,
    }
);

fn sql<R>(q: &Query<R>) -> String {
    rupa_sql::render_query(q, &Postgres).expect("render").sql
}

#[test]
fn a_marker_describes_the_columns_its_equivalent_entity_derives() {
    assert_eq!(Accounts::COLUMNS, AccountEntity::COLUMNS);
    assert_eq!(Accounts::TABLE, AccountEntity::TABLE);
    assert_eq!(Accounts::ID_COLUMNS, AccountEntity::ID_COLUMNS);
}

#[test]
fn a_marker_takes_every_column_attribute_of_the_derive() {
    assert_eq!(Audit::COLUMNS, AuditEntity::COLUMNS);
    assert_eq!(Audit::TABLE, AuditEntity::TABLE);
    assert_eq!(Audit::ID_COLUMNS, AuditEntity::ID_COLUMNS);
    assert_eq!(
        Table::of::<Audit>(),
        Table::of::<AuditEntity>(),
        "equal definitions have equal checksums"
    );
    assert_eq!(Audit::CREATED_AT.name(), "created");
}

#[test]
fn a_marker_names_its_schema_only_when_given_one() {
    assert_eq!(Accounts::TABLE, TableRef::with_schema("main", "accounts"));
    assert_eq!(Memberships::TABLE, TableRef::new("memberships"));
}

#[test]
fn several_ids_declare_a_composite_key() {
    assert_eq!(Memberships::ID_COLUMNS, ["team_id", "user_id"]);
    assert!(
        Memberships::COLUMNS
            .iter()
            .filter(|c| c.primary_key)
            .map(|c| c.name)
            .eq(["team_id", "user_id"])
    );
}

#[test]
fn handles_carry_the_marker_the_table_and_the_kind() {
    let _: Column<Accounts, i64, Scalar> = Accounts::ID;
    let _: Column<Accounts, Option<VarChar<64>>, Scalar> = Accounts::NICK;
    let _: Column<Accounts, Prefs, Json> = Accounts::PREFS;
    let _: Column<Accounts, Option<Prefs>, Json> = Accounts::OLD_PREFS;

    assert_eq!(Accounts::NICK.table(), Some(Accounts::TABLE));
    assert_eq!(Accounts::EMAIL.name(), "email");
}

#[test]
fn a_marker_updates_and_deletes_without_a_row_type() {
    let update = update::<Accounts>()
        .set(Accounts::EMAIL, &"a@b.c".to_owned())
        .set(Accounts::OLD_PREFS, &None)
        .filter(Accounts::ID.gt(10))
        .affected();
    let delete = delete::<Accounts>()
        .filter(Accounts::EMAIL.like("%@old.example"))
        .affected();

    assert_eq!(
        sql(&update),
        r#"UPDATE "main"."accounts" SET "email" = $1, "old_prefs" = $2 WHERE "id" > $3"#
    );
    assert!(sql(&delete).starts_with(r#"DELETE FROM "main"."accounts" WHERE"#));
}
