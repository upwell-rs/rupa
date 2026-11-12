//! `#[query]` on free functions: rendered SQL, and execution on the memory backend.

use std::fmt::Write as _;

use chrono::{DateTime, Utc};
use rupa::Postgres;
use rupa::prelude::*;
use rupa_driver_memory::MemoryDb;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Prefs {
    theme: String,
    beta: bool,
}

#[derive(Debug, Clone, PartialEq, Entity, Insertable)]
#[entity(table = "users", schema = "app")]
pub struct User {
    #[id]
    id: i64,
    email: String,
    nickname: Option<String>,
    active: bool,
    #[column(name = "created")]
    created_at: DateTime<Utc>,
    prefs: Prefs,
}

// The spec's examples.

#[query(filter = email == $email)]
fn by_email(email: &str) -> Option<User>;

#[query(filter = active == $active && prefs.theme == $theme, order_by = created_at desc, limit = 50)]
fn active_by_theme(active: bool, theme: &str) -> Vec<User>;

#[query(
    sql = "SELECT id, email, nickname, active, created, prefs FROM app.users WHERE email = $email"
)]
fn raw_example(email: &str) -> Vec<User>;

// More of the grammar.

#[query(filter = !(nickname is_null) || id in $ids, order_by = id)]
fn named_or_listed(ids: &[i64]) -> Vec<User>;

#[query(filter = active && email like $pattern, order_by = email asc, limit = $n, offset = $skip)]
fn page(pattern: &str, n: u64, skip: u64) -> Vec<User>;

#[query(filter = prefs.beta == $beta && $min_id < id)]
fn beta_after(beta: bool, min_id: i64) -> Vec<User>;

#[query(entity = User, filter = email == $email)]
fn email_taken(email: &str) -> bool;

#[query(filter = id == $id)]
fn exactly(id: i64) -> User;

#[query(sql = "UPDATE app.users SET active = false WHERE id = $id")]
fn deactivate(id: i64) -> u64;

fn pg<R>(q: &Query<R>) -> String {
    let r = rupa::sql::render_query(q, &Postgres).expect("render");
    let mut out = r.sql;
    for (i, p) in r.params.iter().enumerate() {
        let _ = write!(out, "\n${} = {p:?}", i + 1);
    }

    out
}

#[test]
fn rendered_sql() {
    let all = [
        pg(&by_email("a@x")),
        pg(&active_by_theme(true, "dark")),
        pg(&raw_example("a@x")),
        pg(&named_or_listed(&[1, 2])),
        pg(&page("%@x", 10, 20)),
        pg(&beta_after(true, 3)),
        pg(&email_taken("a@x")),
        pg(&exactly(1)),
        pg(&deactivate(1)),
    ];
    insta::assert_snapshot!(all.join("\n---\n"));
}

fn user(id: i64, email: &str, nickname: Option<&str>, active: bool, theme: &str) -> User {
    User {
        id,
        email: email.into(),
        nickname: nickname.map(Into::into),
        active,
        created_at: DateTime::from_timestamp(1_700_000_000 + id, 0).unwrap(),
        prefs: Prefs {
            theme: theme.into(),
            beta: id % 2 == 0,
        },
    }
}

#[test]
fn queries_run_on_memory() {
    let mut db = MemoryDb::new();
    db.register::<User>();
    let users = [
        user(1, "ada@x", Some("ada"), true, "dark"),
        user(2, "bob@x", None, true, "dark"),
        user(3, "cy@y", None, false, "light"),
        user(4, "di@x", Some("di"), true, "light"),
    ];
    insert::<User>()
        .values(&users)
        .affected()
        .execute(&db)
        .unwrap();

    let ids = |v: Vec<User>| v.into_iter().map(|u| u.id).collect::<Vec<_>>();
    assert_eq!(
        by_email("bob@x").execute(&db).unwrap().map(|u| u.id),
        Some(2)
    );
    assert_eq!(
        ids(active_by_theme(true, "dark").execute(&db).unwrap()),
        [2, 1]
    );
    assert_eq!(ids(named_or_listed(&[3]).execute(&db).unwrap()), [1, 3, 4]);
    assert_eq!(ids(page("%@x", 2, 1).execute(&db).unwrap()), [2, 4]);
    assert_eq!(ids(beta_after(true, 1).execute(&db).unwrap()), [2, 4]);
    assert!(email_taken("di@x").execute(&db).unwrap());
    assert!(!email_taken("zed@x").execute(&db).unwrap());
    assert_eq!(exactly(3).execute(&db).unwrap().email, "cy@y");
}
