//! User-defined `#[dsl::function]`s and built-ins, through `#[query]` and
//! `#[repository]`, on the memory backend.

use std::sync::Arc;

use rupa::DynError;
use rupa::core::exec::Executor;
use rupa::dsl::{self, DslError, Expr, Value};
use rupa::prelude::*;
use rupa_driver_memory::MemoryDb;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Prefs {
    theme: String,
    beta: bool,
}

#[derive(Debug, Clone, PartialEq, Entity, Insertable)]
#[entity(table = "users")]
pub struct User {
    #[id]
    id: i64,
    email: String,
    prefs: Prefs,
}

/// A user-defined function: available everywhere (standard SQL), with an
/// `eval` so the memory backend can run it.
#[dsl::function(eval = eval_longer_than)]
pub fn longer_than(s: Expr<String>, n: Expr<i64>) -> Expr<bool> {
    Expr::raw_op(">", Expr::<i64>::call("char_length", [s]), n)
}

fn eval_longer_than(args: &[Value]) -> Result<Value, DslError> {
    match args {
        [Value::Text(s), Value::I64(n)] => Ok(Value::Bool(s.chars().count() as i64 > *n)),
        _ => Ok(Value::Null(rupa::core::SqlType::Bool)),
    }
}

#[repository]
pub trait Users: Send + Sync {
    #[query(filter = dsl::ilike(email, $pattern) && longer_than(email, $n), order_by = id)]
    async fn search(&self, pattern: &str, n: i64) -> Result<Vec<User>, DynError>;

    /// Statically gated (`JsonContainment`); the memory dialect has it.
    #[query(filter = dsl::json_contains(prefs, $part), order_by = id)]
    fn with_prefs(&self, part: serde_json::Value) -> Result<Vec<User>, DynError>;

    #[query(filter = dsl::lower(email) == $email)]
    async fn by_email_ci(&self, email: &str) -> Result<Option<User>, DynError>;
}

fn db() -> MemoryDb {
    let mut db = MemoryDb::new();
    db.register::<User>();
    let users = [
        (1, "Ada@Example.com", "dark", true),
        (2, "bob@x.io", "light", false),
        (3, "ADAM@example.org", "dark", false),
    ]
    .map(|(id, email, theme, beta)| User {
        id,
        email: email.into(),
        prefs: Prefs {
            theme: theme.into(),
            beta,
        },
    });
    db.run(insert::<User>().values(&users).affected()).unwrap();
    db
}

#[test]
fn dsl_functions_in_a_repository() {
    let repo: Arc<dyn Users> = Arc::new(Repo::shared(db()));
    let ids = |v: Vec<User>| v.into_iter().map(|u| u.id).collect::<Vec<_>>();

    let found = pollster::block_on(repo.search("ada%", 15)).unwrap();
    assert_eq!(
        ids(found),
        [3],
        "both match ILIKE, only one is longer than 15"
    );
    assert_eq!(
        ids(repo
            .with_prefs(serde_json::json!({"theme": "dark"}))
            .unwrap()),
        [1, 3]
    );
    assert_eq!(
        ids(repo.with_prefs(serde_json::json!({"beta": true})).unwrap()),
        [1]
    );
    let bob = pollster::block_on(repo.by_email_ci("bob@x.io")).unwrap();
    assert_eq!(bob.map(|u| u.id), Some(2));
}

#[test]
fn dsl_functions_render_for_postgres() {
    #[query(filter = dsl::ilike(email, $pattern) && longer_than(email, $n))]
    fn search(pattern: &str, n: i64) -> Vec<User>;

    let r = rupa::sql::render_query(&search("ada%", 3), &rupa::core::Postgres).unwrap();
    assert_eq!(
        r.sql,
        r#"SELECT "id", "email", "prefs" FROM "users" WHERE "email" ILIKE $1 AND char_length("email") > $2"#
    );
}

/// Runtime-checked functions work with a dialect chosen at run time;
/// statically gated ones would not compile here (see `ui/dsl_static_gate_on_dyn_dialect.rs`).
#[test]
fn runtime_checked_functions_work_with_a_dynamic_dialect() {
    #[repository]
    pub trait Search: Send + Sync {
        #[query(filter = dsl::ilike(email, $pattern), order_by = id)]
        async fn search(&self, pattern: &str) -> Result<Vec<User>, DynError>;
    }
    let boxed = rupa::core::exec::BoxAsyncExecutor::new(db().into_async());
    let repo: Arc<dyn Search> = Arc::new(Repo::shared_async(boxed));
    let found = pollster::block_on(repo.search("%EXAMPLE%")).unwrap();
    assert_eq!(found.iter().map(|u| u.id).collect::<Vec<_>>(), [1, 3]);
}
