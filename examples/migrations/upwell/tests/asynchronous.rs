//! A database on an async pool: sqlx SQLite, with a SQL catalog both modes
//! can run.

mod support;

use std::sync::Arc;

use rupa::DynError;
use rupa::drivers::sqlx::sqlite::SqlitePool;
use rupa::prelude::*;
use rupa::upwell::Database;
use support::{Scratch, source};
use upwell::app;

/// One usage event.
#[derive(Debug, rupa::Entity)]
#[entity(table = "events")]
pub struct Event {
    /// Primary key.
    #[id]
    pub id: i64,
    /// What happened.
    pub kind: String,
}

/// Reads events.
#[repository(database = Events)]
pub trait EventRepository: Send + Sync {
    /// Every event, oldest first.
    #[query(order_by = id)]
    async fn all(&self) -> Result<Vec<Event>, DynError>;
}

/// The events catalog: one SQL migration.
pub mod events {
    use rupa::migrate::{Catalog, Planned, ValidationError, VersionScheme};

    rupa::migrate::sql_migration! {
        /// Creates the events table.
        pub struct CreateEvents {
            version: "1",
            description: "create events",
            up: "events.sql",
        }
    }

    rupa::migrate::migration_catalog! {
        /// The events stream.
        pub enum Migrations, error MigrationsError {
            Events(Planned<CreateEvents>),
        }
    }

    /// The catalog's entries.
    static ENTRIES: [Migrations; 1] = [Migrations::Events(Planned(CreateEvents))];

    /// The events catalog.
    pub fn catalog() -> Result<Catalog<'static, Migrations>, ValidationError> {
        Catalog::new("events", VersionScheme::Numbered, &ENTRIES)
    }
}

rupa::upwell::database! {
    /// Usage events on an async pool.
    pub Events {
        pool: SqlitePool,
        config: "database.events",
        migrations: events,
    }
}

app! {
    /// An application with one async database, discovered like a component.
    pub app EventsApp {
        name: "rupa-example-upwell-async",
        protocol: (),
    }
}

#[tokio::test]
async fn an_async_database_applies_and_provides_its_repositories() {
    let scratch = Scratch::new("async");
    let toml = format!(
        "[database.events]\ndriver = \"sqlx-sqlite\"\npath = {:?}\nparams = {{ mode = \"rwc\" }}\n\n\
         [database.events.migrations]\nstartup = \"apply\"\n",
        scratch.database("events.db")
    );

    let app = EventsApp::builder()
        .expect("the generated builder constructs")
        .config_source(source(&toml))
        .prepare()
        .expect("the application prepares")
        .build()
        .await
        .expect("the application builds after applying migrations");
    let database = app
        .container()
        .resolve::<Arc<Database<Events>>>()
        .await
        .expect("the database resolves")
        .expect("the database is a component");
    let repository = app
        .container()
        .resolve::<Arc<dyn EventRepository>>()
        .await
        .expect("the repository resolves")
        .expect("the database provides the repository");

    insert_event(&database).await;

    let events = repository.all().await.expect("the events table exists");

    assert_eq!(events.len(), 1);
}

/// Inserts one event on the database, which is a connection like its pool.
async fn insert_event(database: &Database<Events>) {
    raw()
        .sql("INSERT INTO events (kind) VALUES ('started')")
        .execute::<u64>()
        .execute(database)
        .await
        .expect("the insert runs");
}
