//! MySQL and SQLite rendering: one snapshot per dialect over the same cases,
//! concentrating on where they differ from Postgres.

#[path = "../../rupa-core/tests/fixtures/mod.rs"]
mod fixtures;

use std::fmt::Write as _;

use fixtures::User;
use rupa_core::ir::{IsolationLevel, TxOptions, TxStatement};
use rupa_core::prelude::*;
use rupa_core::{Capability, Dialect, MySql, Sqlite};
use rupa_sql::{RenderError, render_query, render_tx};

fn show<R>(q: &Query<R>, d: &dyn Dialect) -> String {
    match render_query(q, d) {
        Ok(r) => {
            let mut out = r.sql;
            for (i, p) in r.params.iter().enumerate() {
                let _ = write!(out, "\n  {} = {p:?}", i + 1);
            }
            out
        }
        Err(e) => format!("error: {e:?}"),
    }
}

fn cases(d: &dyn Dialect) -> String {
    let all: Vec<(&str, String)> = vec![
        (
            "select",
            show(&select::<User>().filter(User::EMAIL.eq("a@x")).all(), d),
        ),
        (
            "order by: Postgres null ordering",
            show(
                &select::<User>()
                    .order_by(User::NICKNAME.asc())
                    .order_by(User::ID.desc())
                    .all(),
                d,
            ),
        ),
        (
            "offset without limit",
            show(&select::<User>().offset(5).all(), d),
        ),
        (
            "limit and offset",
            show(&select::<User>().limit(10).offset(5).all(), d),
        ),
        (
            "like",
            show(&select::<User>().filter(User::EMAIL.like("a\\_%")).all(), d),
        ),
        (
            "in / empty in",
            show(
                &select::<User>()
                    .filter(User::ID.in_([1i64, 2]).or(User::ID.in_(Vec::<i64>::new())))
                    .all(),
                d,
            ),
        ),
        (
            "json path as text",
            show(
                &select::<User>()
                    .filter(User::PREFS.path("theme").text().eq("dark"))
                    .all(),
                d,
            ),
        ),
        (
            "json path nested + index + cast",
            show(
                &select::<User>()
                    .filter(
                        User::PREFS
                            .path("a")
                            .index(0)
                            .key("b")
                            .cast::<bool>()
                            .eq(true),
                    )
                    .all(),
                d,
            ),
        ),
        (
            "json path cast to integer",
            show(
                &select::<User>()
                    .filter(User::PREFS.path("n").cast::<i64>().gt(3))
                    .all(),
                d,
            ),
        ),
        (
            "exists",
            show(&select::<User>().filter(User::ACTIVE.eq(true)).exists(), d),
        ),
        (
            "insert",
            show(
                &insert::<User>()
                    .values(&[User::sample(), User::sample()])
                    .affected(),
                d,
            ),
        ),
        (
            "insert returning",
            show(&insert::<User>().value(&User::sample()).returning_one(), d),
        ),
        (
            "update",
            show(&update::<User>().one(&User::sample()).affected(), d),
        ),
        ("delete", show(&delete::<User>().by_id(&7).affected(), d)),
        (
            "json key with a quote",
            show(
                &select::<User>()
                    .filter(User::PREFS.path("it's").text().is_null())
                    .all(),
                d,
            ),
        ),
        (
            "json key with a double quote",
            show(
                &select::<User>()
                    .filter(User::PREFS.path("a\"b").text().is_null())
                    .all(),
                d,
            ),
        ),
    ];
    let mut out = String::new();
    for (name, sql) in all {
        let _ = writeln!(out, "-- {name}\n{sql}\n");
    }
    for (name, stmt) in [
        ("begin", TxStatement::Begin(TxOptions::default())),
        (
            "begin serializable",
            TxStatement::Begin(TxOptions::default().isolation(IsolationLevel::Serializable)),
        ),
        (
            "begin read only",
            TxStatement::Begin(TxOptions::default().read_only()),
        ),
        ("savepoint", TxStatement::Savepoint(1)),
        ("rollback to savepoint", TxStatement::RollbackToSavepoint(1)),
        ("commit", TxStatement::Commit),
    ] {
        let sql = render_tx(&stmt, d).unwrap_or_else(|e| format!("error: {e:?}"));
        let _ = writeln!(out, "-- tx: {name}\n{sql}\n");
    }
    out
}

#[test]
fn mysql() {
    insta::assert_snapshot!(cases(&MySql));
}

#[test]
fn sqlite() {
    insta::assert_snapshot!(cases(&Sqlite));
}

#[test]
fn capabilities_are_enforced() {
    let returning = insert::<User>().value(&User::sample()).returning_one();
    assert_eq!(
        render_query(&returning, &MySql),
        Err(RenderError::UnsupportedCapability(Capability::Returning))
    );
    assert!(render_query(&returning, &Sqlite).is_ok());
    assert_eq!(
        render_tx(
            &TxStatement::Begin(TxOptions::default().read_only()),
            &Sqlite
        ),
        Err(RenderError::UnsupportedCapability(
            Capability::ReadOnlyTransactions
        ))
    );
}

#[test]
fn security_context_rendering() {
    use rupa_core::SecurityContext;
    use rupa_core::ir::Statement;
    let ctx = SecurityContext::new()
        .set("tenant_id", "42")
        .set("user_id", "7")
        .role("app_user");
    let r = rupa_sql::render(&Statement::ApplySecurity(ctx.clone()), &rupa_core::Postgres).unwrap();
    assert_eq!(
        r.sql,
        "SELECT set_config($1, $2, true), set_config($3, $4, true), set_config($5, $6, true)"
    );
    let params: Vec<String> = r.params.iter().map(|p| format!("{p:?}")).collect();
    assert_eq!(
        params,
        [
            r#"Text("app.tenant_id")"#,
            r#"Text("42")"#,
            r#"Text("app.user_id")"#,
            r#"Text("7")"#,
            r#"Text("role")"#,
            r#"Text("app_user")"#
        ]
    );
    for d in [&MySql as &dyn Dialect, &Sqlite] {
        assert_eq!(
            rupa_sql::render(&Statement::ApplySecurity(ctx.clone()), d),
            Err(RenderError::UnsupportedCapability(Capability::NativeRls))
        );
    }
    let bad = SecurityContext::new().set("tenant-id", "1");
    assert_eq!(
        rupa_sql::render(&Statement::ApplySecurity(bad), &rupa_core::Postgres),
        Err(RenderError::InvalidSecurityKey("tenant-id".into()))
    );
}
