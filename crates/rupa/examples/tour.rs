//! A tour of what RUPA can do so far (milestones 1–5).
//!
//! Run with: `cargo run -p rupa --example tour`
//!
//! - Entities and capabilities, derived; typed insert and patch structs.
//! - Queries built with the typed builder: plain values, no IO.
//! - The same query rendered to Postgres SQL with bound parameters...
//! - ...and executed by the in-memory backend, which never sees SQL.
//! - A DSL function: lowered to SQL for Postgres, evaluated in memory.
//! - Transactions: closure and guard APIs, nested savepoints.
//! - An executor chosen at run time (`BoxAsyncExecutor`).
//! - A repository trait shared as `Arc<dyn Trait>`, the way DI holds it.

use chrono::{DateTime, Utc};
use rupa::core::ir::{BinOp, DslFnDef, ExprNode};
use rupa::core::{Dialect, DialectId, DslError, Postgres, Value};
use rupa::prelude::*;
use rupa_driver_memory::MemoryDb;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Entities
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Prefs {
    theme: String,
    beta: bool,
}

/// Each capability is opted into: this entity can be fetched by id,
/// inserted, updated and deleted.
#[derive(Debug, Clone, PartialEq, Entity, Gettable, Insertable, Updatable, Deletable)]
#[entity(table = "users", schema = "app")]
struct User {
    #[id(generated)] // the database assigns it; inserts leave it out
    id: i64,
    email: String,
    nickname: Option<String>,
    active: bool,
    #[column(name = "created")]
    created_at: DateTime<Utc>,
    prefs: Prefs, // not a scalar, but Serialize + Deserialize: a JSON column
}

/// Typed insert input: everything a new user needs, and no id.
#[derive(Insertable)]
#[insertable(entity = User)]
struct NewUser {
    email: String,
    nickname: Option<String>,
    active: bool,
    created_at: DateTime<Utc>,
    prefs: Prefs,
}

/// Partial update: `None` leaves a column alone.
#[derive(Updatable)]
#[updatable(entity = User)]
struct UserPatch {
    #[id]
    id: i64,
    nickname: Option<Option<String>>,
    active: Option<bool>,
}

/// A repository: declared queries, implemented for `Repo<S>`. `async` and
/// receivers are the author's choice; this one is dyn-compatible (the
/// default), so it can be shared as `Arc<dyn UserRepository>`.
#[repository]
trait UserRepository: Send + Sync {
    #[query(filter = email == $email)]
    async fn by_email(&self, email: &str) -> Result<Option<User>, rupa::DynError>;

    /// A plain `fn` in an async repository blocks in place.
    #[query(filter = active == $active && prefs.theme == $theme, order_by = created_at desc)]
    fn active_by_theme(&self, active: bool, theme: &str) -> Result<Vec<User>, rupa::DynError>;
}

/// An entity with no capabilities: `insert::<AuditLog>()` would not compile.
#[derive(Debug, Entity)]
#[entity(table = "audit_log")]
#[allow(dead_code)]
struct AuditLog {
    #[id]
    id: i64,
    message: String,
}

fn new_user(email: &str, nickname: Option<&str>, active: bool, theme: &str, hours: i64) -> NewUser {
    NewUser {
        email: email.into(),
        nickname: nickname.map(Into::into),
        active,
        created_at: DateTime::from_timestamp(1_700_000_000 + hours * 3600, 0).unwrap(),
        prefs: Prefs {
            theme: theme.into(),
            beta: hours % 2 == 0,
        },
    }
}

// ---------------------------------------------------------------------------
// A DSL function: case-insensitive equality
// ---------------------------------------------------------------------------

/// Postgres gets `upper(a) = upper(b)`; the memory backend calls `eval`.
static CI_EQ: DslFnDef = DslFnDef {
    name: "ci_eq",
    lower: lower_ci_eq,
    eval: Some(eval_ci_eq),
};

fn lower_ci_eq(dialect: &dyn Dialect, mut args: Vec<ExprNode>) -> Result<ExprNode, DslError> {
    let (b, a) = (args.pop().unwrap(), args.pop().unwrap());
    match dialect.id() {
        DialectId::Postgres => Ok(ExprNode::Binary(
            BinOp::Eq,
            Box::new(ExprNode::Call("upper", vec![a])),
            Box::new(ExprNode::Call("upper", vec![b])),
        )),
        other => Err(DslError::unsupported("ci_eq", other)),
    }
}

fn eval_ci_eq(args: &[Value]) -> Result<Value, DslError> {
    match args {
        [Value::Text(a), Value::Text(b)] => Ok(Value::Bool(a.eq_ignore_ascii_case(b))),
        _ => Ok(Value::Null(rupa::core::SqlType::Bool)),
    }
}

fn ci_eq(column: Column<User, String, Scalar>, value: &str) -> Expr<bool> {
    Expr::dsl(
        &CI_EQ,
        vec![
            ExprNode::Column(column.column_ref()),
            ExprNode::Param(Value::Text(value.into())),
        ],
    )
}

// ---------------------------------------------------------------------------

fn show_sql<R>(title: &str, query: &Query<R>) {
    let rendered = rupa::sql::render_query(query, &Postgres).expect("render");
    println!("\n── {title}\n   {}", rendered.sql);
    for (i, p) in rendered.params.iter().enumerate() {
        println!("   ${} = {p:?}", i + 1);
    }
}

fn main() {
    // 1. Build queries. Nothing touches a database here.
    let signup = insert::<User>()
        .value(&new_user("ada@example.com", Some("ada"), true, "dark", 1))
        .returning_one();
    let active_dark = select::<User>()
        .filter(col!(User::active).eq(true))
        .filter(col!(User::prefs).path("theme").text().eq("dark"))
        .order_by(col!(User::created_at).desc())
        .limit(10)
        .all();
    let by_email = select::<User>()
        .filter(ci_eq(col!(User::email), "ADA@EXAMPLE.COM"))
        .optional();
    let patch = update::<User>()
        .one(&UserPatch {
            id: 1,
            nickname: Some(None),
            active: None,
        })
        .affected();

    // 2. Render them for Postgres: SQL text plus bound parameters, never interpolated.
    println!("Rendered for Postgres:");
    show_sql(
        "insert from a typed input struct, returning the stored row",
        &signup,
    );
    show_sql("active users with the dark theme", &active_dark);
    show_sql(
        "case-insensitive email lookup (DSL function, lowered)",
        &by_email,
    );
    show_sql("partial update: only the patch's `Some` fields", &patch);

    let db = run_in_memory(signup, active_dark, by_email, patch);
    let db = run_boxed(db);
    run_repository(db);
}

/// 5. A repository, held the way a DI container holds components.
fn run_repository(db: MemoryDb) {
    use std::sync::Arc;

    let users: Arc<dyn UserRepository> = Arc::new(Repo::shared_async(db.into_async()));
    println!(
        "
Repository behind Arc<dyn UserRepository>:"
    );
    let grace = pollster::block_on(users.by_email("grace@example.com")).unwrap();
    println!("   by_email (async) -> {:?}", grace.map(|u| u.id));
    let dark = users.active_by_theme(true, "dark").unwrap();
    println!(
        "   active_by_theme (sync, blocks in place) -> {:?}",
        dark.iter().map(|u| &u.email).collect::<Vec<_>>()
    );
}

/// 3. Run the very same queries on the in-memory backend (sync executor).
fn run_in_memory(
    signup: Query<User>,
    active_dark: Query<Vec<User>>,
    by_email: Query<Option<User>>,
    patch: Query<u64>,
) -> MemoryDb {
    use rupa::core::exec::Executor;
    use rupa::core::tx::Transactional;
    use rupa_driver_memory::MemoryError;

    println!("\nExecuted on the memory backend:");
    let mut db = MemoryDb::new();
    db.register::<User>();

    let ada = db.run(signup).unwrap();
    println!("   inserted {} with generated id {}", ada.email, ada.id);
    let others = [
        new_user("grace@example.com", None, true, "dark", 2),
        new_user("linus@example.com", Some("torvalds"), false, "dark", 3),
        new_user("barbara@example.com", Some("barb"), true, "light", 4),
    ];
    let rest = db
        .run(insert::<User>().values(&others).returning_all())
        .unwrap();
    println!(
        "   inserted {} more, ids {:?}",
        rest.len(),
        rest.iter().map(|u| u.id).collect::<Vec<_>>()
    );

    let found = db.run(active_dark).unwrap();
    println!(
        "   active + dark, newest first: {:?}",
        found.iter().map(|u| &u.email).collect::<Vec<_>>()
    );

    let by_ci = db.run(by_email).unwrap();
    println!(
        "   ci_eq lookup (evaluated, not lowered): {:?}",
        by_ci.map(|u| u.email)
    );

    db.run(patch).unwrap();
    let mut ada = db.run(get::<User>(&1)).unwrap().expect("ada exists");
    println!("   after the patch, ada's nickname is {:?}", ada.nickname);

    // One model: update the whole entity by its id.
    ada.prefs.theme = "light".into();
    db.run(update::<User>().one(&ada).affected()).unwrap();
    println!(
        "   full update -> theme is now {:?}",
        db.run(get::<User>(&1)).unwrap().unwrap().prefs.theme
    );

    // Transactions. The closure form commits on Ok and rolls back on Err;
    // a nested call is a savepoint, so its failure leaves the outer work intact.
    let outcome = db.transaction(|tx| {
        tx.run(
            update::<User>()
                .one(&UserPatch {
                    id: 2,
                    nickname: Some(Some("amazing grace".into())),
                    active: None,
                })
                .affected(),
        )?;
        let nested: Result<(), MemoryError> = tx.transaction(|inner| {
            inner.run(delete::<User>().by_id(&1).affected())?;
            Err(MemoryError::Unsupported("changed my mind".into()))
        });
        println!("   nested transaction rolled back: {}", nested.is_err());
        Ok::<_, MemoryError>(())
    });
    outcome.unwrap();
    println!(
        "   after commit: grace is {:?}, ada still exists: {}",
        db.run(get::<User>(&2)).unwrap().unwrap().nickname,
        db.run(get::<User>(&1)).unwrap().is_some()
    );

    // The guard form: a dropped `Tx` rolls back.
    {
        let mut tx = db.begin().unwrap();
        tx.run(delete::<User>().filter(col!(User::id).gt(0)).affected())
            .unwrap();
    }
    println!(
        "   dropped tx (deleted everything) -> {} users still there",
        db.run(select::<User>().all()).unwrap().len()
    );

    let removed = db
        .run(
            delete::<User>()
                .filter(col!(User::active).eq(false))
                .affected(),
        )
        .unwrap();
    println!("   deleted {removed} inactive user(s)");

    // Result shapes are checked: `.one()` on several rows is an error, not a silent pick.
    let err = db.run(select::<User>().one()).unwrap_err();
    println!("   .one() over several rows -> {err}");

    db
}

/// 4. An executor chosen at run time: boxed, dialect known only dynamically.
fn run_boxed(db: MemoryDb) -> MemoryDb {
    use rupa::core::exec::{AsyncExecutor, BoxAsyncExecutor, ExecError};

    let mut dynamic = BoxAsyncExecutor::new(db.clone().into_async());
    println!(
        "\nBoxed async executor, dialect {:?}:",
        dynamic.dialect().id()
    );
    let count = pollster::block_on(dynamic.run(select::<User>().all()))
        .unwrap()
        .len();
    println!("   {count} user(s) remain");
    let missing =
        pollster::block_on(dynamic.run(select::<User>().filter(col!(User::id).eq(99)).one()))
            .unwrap_err();
    println!(
        "   missing row -> {:?} (inspectable through any driver)",
        missing.result_error()
    );
    db
}
