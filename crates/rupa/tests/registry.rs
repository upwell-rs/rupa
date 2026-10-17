//! `rupa::registry()` / `rupa::connect()`: drivers come from cargo features.
//! Run with `--features full`.
#![cfg(all(
    feature = "sqlite",
    feature = "memory",
    feature = "postgres",
    feature = "postgres-sync",
    feature = "mysql"
))]

use rupa::core::{Dialect, DialectId};
use rupa::{DbConfig, DriverConfig, PostgresOptions};

#[test]
fn the_registry_is_built_once() {
    assert!(std::ptr::eq(rupa::registry(), rupa::registry()));
    let names: Vec<_> = rupa::registry().drivers().map(|d| d.name()).collect();
    assert_eq!(
        names,
        ["postgres", "postgres-sync", "sqlite", "mysql", "memory"]
    );
}

#[test]
fn schemes_and_names_resolve() {
    let resolve = |c: DbConfig| rupa::registry().resolve(&c).map(|d| d.name()).unwrap();
    assert_eq!(
        resolve(DbConfig::new("postgres://h/db")),
        "postgres",
        "async first"
    );
    assert_eq!(
        resolve(
            DbConfig::new("postgresql://h/db")
                .driver(DriverConfig::PostgresSync(PostgresOptions::default()))
        ),
        "postgres-sync"
    );
    assert_eq!(resolve(DbConfig::new("sqlite::memory:")), "sqlite");
    assert_eq!(resolve(DbConfig::new("mysql://h/db")), "mysql");
    assert_eq!(resolve(DbConfig::new("memory:")), "memory");
}

#[test]
fn connect_without_a_server() {
    for (url, dialect) in [
        ("sqlite::memory:", DialectId::Sqlite),
        ("memory:", DialectId::Memory),
    ] {
        let db = pollster::block_on(rupa::connect(&DbConfig::new(url))).unwrap();
        assert_eq!(db.dialect().id(), dialect);
    }
    let err = pollster::block_on(rupa::connect(&DbConfig::new("oracle://h"))).unwrap_err();
    assert!(
        err.to_string()
            .contains("no registered driver accepts `oracle:`"),
        "{err}"
    );
}
