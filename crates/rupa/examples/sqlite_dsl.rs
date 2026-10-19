//! Custom DSL function, JSON path and dialect-specific SQL rendering.
//!
//! Run: `cargo run -p rupa --example sqlite_dsl --features sqlite`

#[path = "support/sqlite.rs"]
mod sqlite;

use rupa::core::{Dialect, DialectId, DslError};
use rupa::prelude::*;
use rupa::{Executor, Pool};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Settings {
    theme: String,
}

#[derive(Debug, Clone, Entity, Insertable)]
#[entity(table = "profiles")]
struct Profile {
    #[id]
    id: i64,
    name: String,
    settings: Settings,
}

#[rupa::dsl::function]
fn same_name_ci(
    dialect: &dyn Dialect,
    left: Expr<String>,
    right: Expr<String>,
) -> Result<Expr<bool>, DslError> {
    match dialect.id() {
        DialectId::Sqlite | DialectId::Postgres | DialectId::MySql => Ok(Expr::raw_op(
            "=",
            Expr::<String>::call("lower", [left]),
            Expr::<String>::call("lower", [right]),
        )),
        other => Err(DslError::unsupported("same_name_ci", other)),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pool = sqlite::open(
        "CREATE TABLE profiles (id INTEGER PRIMARY KEY, name TEXT NOT NULL, settings TEXT NOT NULL)",
    )?;
    let mut conn = pool.get()?;
    conn.run(
        insert::<Profile>()
            .value(&Profile {
                id: 1,
                name: "Ada".into(),
                settings: Settings {
                    theme: "dark".into(),
                },
            })
            .affected(),
    )?;

    let query = select::<Profile>()
        .filter(same_name_ci(Profile::NAME, "ADA"))
        .filter(Profile::SETTINGS.path("theme").text().eq("dark"))
        .all();
    let rendered = rupa::sql::render_query(&query, &rupa::core::Sqlite)?;
    assert_eq!(rendered.params.len(), 2);
    assert!(!rendered.sql.contains("'ADA'"));
    assert_eq!(conn.run(query)?[0].id, 1);

    println!("custom DSL, JSON path and bound parameters passed");
    Ok(())
}
