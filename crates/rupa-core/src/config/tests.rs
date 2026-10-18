use std::time::Duration;

use serde_json::json;

use super::*;

fn parse(v: serde_json::Value) -> Result<DbConfig, String> {
    serde_json::from_value(v).map_err(|e| e.to_string())
}

#[test]
fn schemes() {
    assert_eq!(
        DbConfig::new("postgres://h/db").scheme().as_deref(),
        Some("postgres")
    );
    assert_eq!(
        DbConfig::new("sqlite::memory:").scheme().as_deref(),
        Some("sqlite")
    );
    assert_eq!(
        DbConfig::new("SQLite://x.db").scheme().as_deref(),
        Some("sqlite")
    );
    assert_eq!(DbConfig::new("memory:").scheme().as_deref(), Some("memory"));
    assert_eq!(DbConfig::new("no scheme").scheme(), None);
}

#[test]
fn passwords_are_redacted() {
    let c = DbConfig::new("postgres://app:s3cr@t@db:5432/app?sslmode=require");
    assert_eq!(
        c.redacted_url(),
        "postgres://app:***@db:5432/app?sslmode=require"
    );
    assert!(!format!("{c:?}").contains("s3cr"));
    for unchanged in [
        "postgres://app@db/x",
        "sqlite://data/a.db",
        "sqlite::memory:",
    ] {
        assert_eq!(redact(unchanged), unchanged);
    }
}

#[test]
fn durations() {
    for (text, ms) in [
        ("250ms", 250),
        ("30s", 30_000),
        ("1.5s", 1500),
        ("2m", 120_000),
        ("1h", 3_600_000),
        ("40", 40),
    ] {
        let d = Duration::from_millis(ms);
        assert_eq!(parse_duration(text), Ok(d), "{text}");
        assert_eq!(parse_duration(&format_duration(d)), Ok(d));
    }
    assert!(parse_duration("5 parsecs").is_err());
    assert!(parse_duration("").is_err());
}

#[test]
fn shared_fields_and_pool() {
    let c = parse(json!({
        "url": "sqlite::memory:",
        "qualifier": "primary",
        "pool": { "max_size": 4, "acquire_timeout": "5s", "idle_timeout": null, "max_lifetime": "off" },
    }))
    .unwrap();
    assert_eq!(c.qualifier.as_deref(), Some("primary"));
    assert_eq!(c.pool.max_size, 4);
    assert_eq!(c.pool.acquire_timeout, Some(Duration::from_secs(5)));
    assert_eq!(c.pool.idle_timeout, None);
    assert_eq!(c.pool.max_lifetime, None);
    assert_eq!(c.driver, DriverConfig::Auto, "no tag, no options");
    assert!(parse(json!({ "url": "x:", "pool": { "max": 1 } })).is_err());
}

#[test]
fn tagged_driver_options_are_typed() {
    let c = parse(json!({
        "url": "sqlite://app.db",
        "driver": "sqlite",
        "busy_timeout": "1s",
        "journal_mode": "wal",
    }))
    .unwrap();
    assert_eq!(
        c.driver,
        DriverConfig::Sqlite(SqliteOptions {
            busy_timeout: Some(Duration::from_secs(1)),
            foreign_keys: true,
            journal_mode: Some("wal".into()),
            statement_cache_size: None,
        })
    );
    let err = parse(json!({ "url": "sqlite://a.db", "driver": "sqlite", "busy": "1s" }));
    assert!(
        err.unwrap_err().contains("busy"),
        "unknown options are refused"
    );
    let err = parse(json!({ "url": "memory:", "driver": "memory", "x": 1 }));
    assert!(err.is_err());
}

#[test]
fn untagged_options_infer_the_driver_from_the_scheme() {
    let c = parse(json!({ "url": "postgres://h/db", "application_name": "api" })).unwrap();
    assert_eq!(
        c.driver,
        DriverConfig::Postgres(PostgresOptions {
            application_name: Some("api".into()),
            connect_timeout: None,
            statement_cache_size: None,
        })
    );
    assert!(parse(json!({ "url": "foo://h", "x": 1 })).is_err());
}

#[test]
fn unknown_drivers_keep_their_options_as_strings() {
    let c = parse(json!({ "url": "duck://x", "driver": "duckdb", "threads": 4, "mode": "ro" }))
        .unwrap();
    let DriverConfig::Other { name, options } = &c.driver else {
        panic!("{c:?}")
    };
    assert_eq!(name, "duckdb");
    assert_eq!(options["threads"], "4");
    assert_eq!(options["mode"], "ro");
}

#[test]
fn round_trips() {
    for c in [
        DbConfig::new("sqlite::memory:"),
        DbConfig::new("mysql://h/db")
            .qualifier("replica")
            .driver(MySqlOptions {
                statement_cache_size: Some(64),
                init: vec!["SET time_zone = '+00:00'".into()],
            }),
        DbConfig::new("postgres://h/db").driver(DriverConfig::PostgresSync(PostgresOptions {
            application_name: None,
            connect_timeout: Some(Duration::from_secs(3)),
            statement_cache_size: Some(0),
        })),
        DbConfig::new("memory:").driver(DriverConfig::Memory),
    ] {
        let value = serde_json::to_value(&c).unwrap();
        assert_eq!(parse(value.clone()).unwrap(), c, "{value}");
    }
}
