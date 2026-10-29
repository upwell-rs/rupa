use super::*;

fn postgres() -> DatabaseSettings {
    DatabaseSettings {
        driver: Some(String::from("postgres")),
        host: Some(String::from("db.internal")),
        port: Some(5432),
        user: Some(String::from("app")),
        password: Some(String::from("p@ss word")),
        name: Some(String::from("app")),
        params: BTreeMap::from([(String::from("sslmode"), String::from("require"))]),
        ..DatabaseSettings::default()
    }
}

#[test]
fn separate_fields_build_an_encoded_url() {
    let runtime = postgres()
        .runtime()
        .expect("the fields describe a connection");

    assert_eq!(
        runtime.url,
        "postgres://app:p%40ss%20word@db.internal:5432/app?sslmode=require"
    );
}

#[test]
fn sqlite_takes_a_path_and_rejects_server_fields() {
    let file = DatabaseSettings {
        driver: Some(String::from("sqlite")),
        path: Some(String::from("data/app.db")),
        ..DatabaseSettings::default()
    };
    let memory = DatabaseSettings {
        path: Some(String::from(":memory:")),
        ..file.clone()
    };
    let created = DatabaseSettings {
        params: BTreeMap::from([(String::from("mode"), String::from("rwc"))]),
        ..file.clone()
    };
    let with_host = DatabaseSettings {
        host: Some(String::from("db")),
        ..file.clone()
    };

    assert_eq!(file.runtime().expect("a path").url, "sqlite://data/app.db");
    assert_eq!(memory.runtime().expect("memory").url, "sqlite::memory:");
    assert_eq!(
        created.runtime().expect("a path with parameters").url,
        "sqlite://data/app.db?mode=rwc"
    );
    assert!(matches!(
        with_host.runtime(),
        Err(SettingsError::Unused { field: "host", .. })
    ));
}

#[test]
fn a_url_overrides_the_fields() {
    let settings = DatabaseSettings {
        url: Some(String::from("postgres://other@elsewhere/db")),
        ..postgres()
    };

    assert_eq!(
        settings.runtime().expect("the override").url,
        "postgres://other@elsewhere/db"
    );
}

#[test]
fn migrations_use_their_own_credentials() {
    let fields = DatabaseSettings {
        migrations: MigrationSettings {
            user: Some(String::from("migrator")),
            password: Some(String::from("s3cret")),
            ..MigrationSettings::default()
        },
        ..postgres()
    };
    let overridden = DatabaseSettings {
        url: Some(String::from("postgres://app:pw@db.internal/app")),
        ..fields.clone()
    };
    let shared = postgres();

    assert_eq!(
        fields.migration().expect("migration credentials").url,
        "postgres://migrator:s3cret@db.internal:5432/app?sslmode=require"
    );
    assert_eq!(
        overridden
            .migration()
            .expect("credentials in the override")
            .url,
        "postgres://migrator:s3cret@db.internal/app"
    );
    assert_eq!(
        shared.migration().expect("the runtime connection").url,
        shared.runtime().expect("the runtime connection").url
    );
}

#[test]
fn a_driver_or_url_is_required() {
    let settings = DatabaseSettings {
        driver: None,
        ..postgres()
    };

    assert!(matches!(settings.runtime(), Err(SettingsError::NoDriver)));
}

#[test]
fn debug_output_hides_passwords() {
    let debug = format!(
        "{:?}",
        DatabaseSettings {
            url: Some(String::from("postgres://app:hunter2@db/app")),
            ..postgres()
        }
    );

    assert!(!debug.contains("p@ss word"));
    assert!(!debug.contains("hunter2"));
}

#[test]
fn the_schema_declares_every_field_for_tooling() {
    let upwell::schema::ConfigShape::Struct(fields) = DatabaseSettings::SCHEMA.shape else {
        panic!("DatabaseSettings declares a struct schema");
    };
    let names: Vec<_> = fields.iter().map(|field| field.name).collect();
    let required: Vec<_> = fields
        .iter()
        .filter(|field| field.required)
        .map(|field| field.name)
        .collect();
    let upwell::schema::ConfigShape::Struct(migrations) = MigrationSettings::SCHEMA.shape else {
        panic!("MigrationSettings declares a struct schema");
    };

    assert_eq!(
        names,
        [
            "driver",
            "host",
            "port",
            "user",
            "password",
            "name",
            "path",
            "params",
            "url",
            "pool",
            "options",
            "migrations"
        ]
    );
    assert_eq!(required, Vec::<&str>::new(), "every field has a default");
    assert_eq!(
        migrations
            .iter()
            .map(|field| field.name)
            .collect::<Vec<_>>(),
        ["startup", "user", "password", "url"]
    );
    assert!(fields.iter().all(|field| !field.docs.is_empty()));
}

#[test]
fn private_memory_databases_refuse_startup_migrations() {
    let memory = |path: &str, startup| DatabaseSettings {
        driver: Some(String::from("sqlite")),
        path: Some(path.to_owned()),
        migrations: MigrationSettings {
            startup,
            ..MigrationSettings::default()
        },
        ..DatabaseSettings::default()
    };
    let by_url = |url: &str| DatabaseSettings {
        url: Some(url.to_owned()),
        ..DatabaseSettings::default()
    };

    assert!(matches!(
        memory(":memory:", Startup::Check).startup(),
        Err(SettingsError::PrivateMemory)
    ));
    assert!(matches!(
        memory(":memory:", Startup::Apply).startup(),
        Err(SettingsError::PrivateMemory)
    ));
    assert!(matches!(
        by_url("sqlite://:memory:?mode=rwc").startup(),
        Err(SettingsError::PrivateMemory)
    ));
    assert_eq!(
        memory(":memory:", Startup::Off)
            .startup()
            .expect("off never connects"),
        Startup::Off
    );
    assert_eq!(
        memory("app.db", Startup::Apply)
            .startup()
            .expect("a file is shared by every connection"),
        Startup::Apply
    );
    assert_eq!(
        by_url("sqlite://memory.db")
            .startup()
            .expect("a file named like memory is still a file"),
        Startup::Check
    );
}
