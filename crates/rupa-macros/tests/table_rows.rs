//! `rupa::table!(.. as Row { .. })`: a table that also declares its row type.

use rupa::core::schema::Table;
use rupa::prelude::*;
use rupa::{Postgres, VarChar};

rupa::table!(
    /// A table that is also read.
    Accounts = "main"."accounts" as AccountRow {
        /// The surrogate key.
        #[id(generated)]
        id: i64,
        #[column(name = "mail")]
        email: String,
        nick: Option<VarChar<64>>,
    }
);

rupa::table!(
    Plain = "main"."accounts" {
        #[id(generated)]
        id: i64,
        #[column(name = "mail")]
        email: String,
        nick: Option<VarChar<64>>,
    }
);

#[derive(Entity)]
#[entity(table = "accounts", schema = "main")]
struct Derived {
    #[id(generated)]
    id: i64,
    #[column(name = "mail")]
    email: String,
    nick: Option<VarChar<64>>,
}

fn sql<R>(q: &Query<R>) -> String {
    rupa_sql::render_query(q, &Postgres).expect("render").sql
}

fn row() -> AccountRow {
    AccountRow {
        id: 7,
        email: "a@b.c".into(),
        nick: None,
    }
}

#[test]
fn the_marker_and_the_row_are_one_type() {
    let _: rupa::Column<AccountRow, i64, rupa::Scalar> = Accounts::ID;

    assert_eq!(AccountRow::EMAIL.name(), Accounts::EMAIL.name());
}

#[test]
fn the_row_form_describes_what_the_marker_and_the_derive_do() {
    assert_eq!(Accounts::COLUMNS, Plain::COLUMNS);
    assert_eq!(Accounts::COLUMNS, Derived::COLUMNS);
    assert_eq!(Accounts::TABLE, Derived::TABLE);
    assert_eq!(Accounts::ID_COLUMNS, Plain::ID_COLUMNS);
    assert_eq!(Table::of::<Accounts>(), Table::of::<Plain>());
    assert_eq!(Table::of::<Accounts>(), Table::of::<Derived>());
}

#[test]
fn rows_select_get_update_and_delete() {
    let select = select::<Accounts>().filter(Accounts::NICK.is_null()).all();
    let get = get::<Accounts>(&7);
    let delete_by_id = delete::<Accounts>().by_id(&7).affected();
    let update = update::<Accounts>().one(&row()).affected();
    let delete = delete::<Accounts>().one(&row()).affected();
    let insert = insert::<Accounts>().value(&row()).affected();

    assert!(sql(&select).contains(r#"FROM "main"."accounts""#));
    assert!(sql(&get).contains(r#"WHERE "id" = $1"#));
    assert!(sql(&delete_by_id).starts_with("DELETE"));
    assert!(sql(&update).starts_with(r#"UPDATE "main"."accounts" SET "mail""#));
    assert!(sql(&delete).starts_with("DELETE"));
    assert!(sql(&insert).starts_with("INSERT"));
}

#[test]
fn the_row_form_still_writes_through_set_and_filters() {
    let update = update::<Accounts>()
        .set(Accounts::NICK, &None)
        .filter(Accounts::ID.gt(1))
        .affected();

    assert!(sql(&update).contains(r#"SET "nick" = $1 WHERE "id" > $2"#));
}

rupa::table!(
    Notes = "notes" as NoteRow {
        #[id]
        id: i64,
        #[column(references = Accounts, relation = author)]
        author_id: i64,
    }
);

#[test]
fn the_row_form_takes_relations() {
    let join = select::<Notes>().join::<Accounts>().inner().all();

    assert!(sql(&join).contains("INNER JOIN"));
    let _ = NoteRow::author();
}

rupa::table!(Authors = "authors" { #[id] id: i64 });

rupa::table!(
    Posts = "posts" as PostRow {
        #[id]
        id: i64,
        #[column(references = Authors)]
        author_id: i64,
        #[column(references = Accounts)]
        account_id: Option<i64>,
    }
);

rupa::table!(
    Comments = "comments" {
        #[id]
        id: i64,
        #[column(references = Posts)]
        post_id: i64,
    }
);

#[derive(Entity)]
#[entity(table = "reviews")]
struct Review {
    #[id]
    id: i64,
    #[column(references = Authors)]
    author_id: i64,
}

#[test]
fn references_cross_every_form() {
    let post = Table::of::<Posts>();
    let comment = Table::of::<Comments>();
    let review = Table::of::<Review>();

    assert_eq!(post.foreign_keys.len(), 2);
    assert_eq!(comment.foreign_keys[0].references.table, Posts::TABLE);
    assert_eq!(review.foreign_keys[0].references.table, Authors::TABLE);
}
