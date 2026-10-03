//! A tour of what RUPA can do so far (milestones 1–2).
//!
//! Run with: `cargo run -p rupa --example tour`
//!
//! - An entity, written by hand (`#[derive(Entity)]` arrives in milestone 3).
//! - Queries built with the typed builder: plain values, no IO.
//! - The same query rendered to Postgres SQL with bound parameters...
//! - ...and executed by the in-memory backend, which never sees SQL.
//! - A DSL function: lowered to SQL for Postgres, evaluated in memory.
//! - An executor chosen at run time (`BoxAsyncExecutor`).

use chrono::{DateTime, Utc};
use rupa::core::column::{JsonCodec, ScalarCodec, column_meta};
use rupa::core::exec::{AsyncExecutor, BoxAsyncExecutor, ExecError, Executor};
use rupa::core::ir::{BinOp, DslFnDef, ExprNode, TableRef};
use rupa::core::prelude::*;
use rupa::core::{ColumnMeta, Dialect, DialectId, DslError, Postgres, ResultError, Row, Value};
use rupa_driver_memory::MemoryDb;
use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// The entity
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Prefs {
    theme: String,
    beta: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct User {
    id: i64,
    email: String,
    nickname: Option<String>,
    active: bool,
    created_at: DateTime<Utc>,
    prefs: Prefs, // stored as JSON
}

impl User {
    // Typed column handles. The kind (scalar or JSON) is part of the type,
    // so `User::EMAIL.path("x")` would not compile.
    const ID: Column<User, i64, Scalar> = Column::scalar("id");
    const EMAIL: Column<User, String, Scalar> = Column::scalar("email");
    const NICKNAME: Column<User, Option<String>, Scalar> = Column::scalar("nickname");
    const ACTIVE: Column<User, bool, Scalar> = Column::scalar("active");
    const CREATED_AT: Column<User, DateTime<Utc>, Scalar> = Column::scalar("created");
    const PREFS: Column<User, Prefs, Json> = Column::json("prefs");
}

static USER_COLUMNS: [ColumnMeta; 6] = [
    column_meta::<i64, ScalarCodec<i64>>("id", "id"),
    column_meta::<String, ScalarCodec<String>>("email", "email"),
    column_meta::<Option<String>, ScalarCodec<Option<String>>>("nickname", "nickname"),
    column_meta::<bool, ScalarCodec<bool>>("active", "active"),
    column_meta::<DateTime<Utc>, ScalarCodec<DateTime<Utc>>>("created_at", "created"),
    column_meta::<Prefs, JsonCodec<Prefs>>("prefs", "prefs"),
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
        ]
    }
}

fn user(id: i64, email: &str, nickname: Option<&str>, active: bool, theme: &str) -> User {
    User {
        id,
        email: email.into(),
        nickname: nickname.map(Into::into),
        active,
        created_at: DateTime::from_timestamp(1_700_000_000 + id * 3600, 0).unwrap(),
        prefs: Prefs {
            theme: theme.into(),
            beta: id % 2 == 0,
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
    let active_dark = select::<User>()
        .filter(User::ACTIVE.eq(true))
        .filter(User::PREFS.path("theme").text().eq("dark"))
        .order_by(User::CREATED_AT.desc())
        .limit(10)
        .all();
    let by_email = select::<User>()
        .filter(ci_eq(User::EMAIL, "ADA@EXAMPLE.COM"))
        .optional();
    let deactivate = update::<User>()
        .set(User::ACTIVE, &false)
        .filter(User::NICKNAME.is_null())
        .affected();

    // 2. Render them for Postgres: SQL text plus bound parameters, never interpolated.
    println!("Rendered for Postgres:");
    show_sql("active users with the dark theme", &active_dark);
    show_sql(
        "case-insensitive email lookup (DSL function, lowered)",
        &by_email,
    );
    show_sql("deactivate users without a nickname", &deactivate);

    // 3. Run the very same queries on the in-memory backend.
    println!("\nExecuted on the memory backend:");
    let mut db = MemoryDb::new();
    db.register::<User>();
    let users = [
        user(1, "ada@example.com", Some("ada"), true, "dark"),
        user(2, "grace@example.com", None, true, "dark"),
        user(3, "linus@example.com", Some("torvalds"), false, "dark"),
        user(4, "barbara@example.com", Some("barb"), true, "light"),
    ];
    let inserted = Executor::run(&mut db, insert::<User>().values(&users).affected()).unwrap();
    println!("   inserted {inserted} users");

    let found = Executor::run(&mut db, active_dark).unwrap();
    println!(
        "   active + dark, newest first: {:?}",
        found.iter().map(|u| &u.email).collect::<Vec<_>>()
    );

    let ada = Executor::run(&mut db, by_email).unwrap();
    println!(
        "   ci_eq lookup (evaluated, not lowered): {:?}",
        ada.map(|u| u.email)
    );

    let changed = Executor::run(&mut db, deactivate).unwrap();
    println!("   deactivated {changed} user(s) without a nickname");

    // Result shapes are checked: `.one()` on several rows is an error, not a silent pick.
    let err = Executor::run(
        &mut db,
        select::<User>().filter(User::ACTIVE.eq(true)).one(),
    )
    .unwrap_err();
    println!("   .one() over several rows -> {err}");

    // 4. An executor chosen at run time: boxed, dialect known only dynamically.
    let mut dynamic = BoxAsyncExecutor::new(db);
    println!(
        "\nBoxed async executor, dialect {:?}:",
        dynamic.dialect().id()
    );
    let count =
        pollster::block_on(dynamic.run(select::<User>().filter(User::ACTIVE.eq(true)).all()))
            .unwrap()
            .len();
    println!("   {count} active user(s) remain");
    let missing = pollster::block_on(dynamic.run(select::<User>().filter(User::ID.eq(99)).one()))
        .unwrap_err();
    println!(
        "   missing row -> {:?} (inspectable through any driver)",
        missing.result_error()
    );
}
